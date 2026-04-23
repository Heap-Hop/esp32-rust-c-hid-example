//! Tiny line-oriented text protocol for driving the HID bridge from `nc`.
//!
//! Grammar (one command per line, whitespace-separated tokens, ASCII):
//!
//! ```text
//! key  <name>            # tap a key (press + release)
//! k    <name>            # short alias for `key`
//!
//! mouse <dx> <dy>        # relative mouse movement, -127..127 per axis
//! m     <dx> <dy>
//!
//! click [left|right|middle]  # mouse button click (default: left)
//! c     [left|right|middle]
//!
//! media <name>           # consumer-control key tap (volume, play/pause, …)
//! md    <name>
//!
//! help                   # print this table of commands
//! ```
//!
//! Unknown commands, too few arguments, or out-of-range numbers yield an
//! `Invalid` command that the caller turns into a single-line error reply.

use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Key(&'static KeySpec),
    Mouse { dx: i8, dy: i8 },
    Click(MouseButton),
    Media(&'static MediaSpec),
    Help,
    Invalid(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

impl MouseButton {
    pub fn bit(self) -> u8 {
        match self {
            Self::Left => 1 << 0,
            Self::Right => 1 << 1,
            Self::Middle => 1 << 2,
        }
    }
}

/// A named key on the boot keyboard page.
#[derive(Debug, Clone)]
pub struct KeySpec {
    pub name: &'static str,
    /// Modifier byte (left-ctrl bit 0, left-shift bit 1, etc.).
    pub modifier: u8,
    /// HID usage ID on Usage Page 0x07 (Keyboard/Keypad).
    pub keycode: u8,
}

impl PartialEq for KeySpec {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

impl Eq for KeySpec {}

/// A named consumer-control (media) key.
#[derive(Debug, Clone)]
pub struct MediaSpec {
    pub name: &'static str,
    /// HID usage ID on Usage Page 0x0c (Consumer).
    pub usage_code: u16,
}

impl PartialEq for MediaSpec {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

impl Eq for MediaSpec {}

// ── HID usage ID constants ───────────────────────────────────────────────────
// Usage Page 0x07 (Keyboard / Keypad).
const HID_KEY_A: u8 = 0x04;
const HID_KEY_1: u8 = 0x1e;
const HID_KEY_ENTER: u8 = 0x28;
const HID_KEY_ESCAPE: u8 = 0x29;
const HID_KEY_BACKSPACE: u8 = 0x2a;
const HID_KEY_TAB: u8 = 0x2b;
const HID_KEY_SPACE: u8 = 0x2c;
const HID_KEY_RIGHT_ARROW: u8 = 0x4f;
const HID_KEY_LEFT_ARROW: u8 = 0x50;
const HID_KEY_DOWN_ARROW: u8 = 0x51;
const HID_KEY_UP_ARROW: u8 = 0x52;

/// Named key table. Lookup is case-insensitive.
pub const NAMED_KEYS: &[KeySpec] = &[
    KeySpec { name: "enter", modifier: 0, keycode: HID_KEY_ENTER },
    KeySpec { name: "return", modifier: 0, keycode: HID_KEY_ENTER },
    KeySpec { name: "escape", modifier: 0, keycode: HID_KEY_ESCAPE },
    KeySpec { name: "esc", modifier: 0, keycode: HID_KEY_ESCAPE },
    KeySpec { name: "backspace", modifier: 0, keycode: HID_KEY_BACKSPACE },
    KeySpec { name: "tab", modifier: 0, keycode: HID_KEY_TAB },
    KeySpec { name: "space", modifier: 0, keycode: HID_KEY_SPACE },
    KeySpec { name: "left", modifier: 0, keycode: HID_KEY_LEFT_ARROW },
    KeySpec { name: "right", modifier: 0, keycode: HID_KEY_RIGHT_ARROW },
    KeySpec { name: "up", modifier: 0, keycode: HID_KEY_UP_ARROW },
    KeySpec { name: "down", modifier: 0, keycode: HID_KEY_DOWN_ARROW },
];

// Usage Page 0x0c (Consumer).
const HID_CONSUMER_PLAY_PAUSE: u16 = 0x00cd;
const HID_CONSUMER_SCAN_NEXT_TRACK: u16 = 0x00b5;
const HID_CONSUMER_SCAN_PREVIOUS_TRACK: u16 = 0x00b6;
const HID_CONSUMER_STOP: u16 = 0x00b7;
const HID_CONSUMER_MUTE: u16 = 0x00e2;
const HID_CONSUMER_VOLUME_INCREMENT: u16 = 0x00e9;
const HID_CONSUMER_VOLUME_DECREMENT: u16 = 0x00ea;

pub const NAMED_MEDIA_KEYS: &[MediaSpec] = &[
    MediaSpec { name: "play", usage_code: HID_CONSUMER_PLAY_PAUSE },
    MediaSpec { name: "pause", usage_code: HID_CONSUMER_PLAY_PAUSE },
    MediaSpec { name: "playpause", usage_code: HID_CONSUMER_PLAY_PAUSE },
    MediaSpec { name: "next", usage_code: HID_CONSUMER_SCAN_NEXT_TRACK },
    MediaSpec { name: "prev", usage_code: HID_CONSUMER_SCAN_PREVIOUS_TRACK },
    MediaSpec { name: "previous", usage_code: HID_CONSUMER_SCAN_PREVIOUS_TRACK },
    MediaSpec { name: "stop", usage_code: HID_CONSUMER_STOP },
    MediaSpec { name: "mute", usage_code: HID_CONSUMER_MUTE },
    MediaSpec { name: "volup", usage_code: HID_CONSUMER_VOLUME_INCREMENT },
    MediaSpec { name: "voldown", usage_code: HID_CONSUMER_VOLUME_DECREMENT },
];

/// A short cheat-sheet, returned by the `help` command.
pub const HELP_TEXT: &str = "\
commands (one per line, space-separated):
  key|k <name>          tap a key: letter (a-z), digit (0-9), or named
                        (enter, esc, space, tab, backspace, left, right, up, down)
  mouse|m <dx> <dy>     move mouse relatively, each axis -127..127
  click|c [left|right|middle]   mouse button click, default left
  media|md <name>       play, pause, playpause, next, prev, stop,
                        mute, volup, voldown
  help                  print this help";

pub fn parse(line: &str) -> Command {
    let mut tokens = line.split_whitespace();
    let Some(verb) = tokens.next() else {
        return Command::Invalid("empty command".to_owned());
    };
    let verb = verb.to_ascii_lowercase();
    let args: Vec<&str> = tokens.collect();

    match verb.as_str() {
        "key" | "k" => parse_key(&args),
        "mouse" | "m" => parse_mouse(&args),
        "click" | "c" => parse_click(&args),
        "media" | "md" => parse_media(&args),
        "help" | "?" => Command::Help,
        other => Command::Invalid(format!(
            "unknown command '{other}' (try 'help')"
        )),
    }
}

fn parse_key(args: &[&str]) -> Command {
    let [name] = args else {
        return Command::Invalid(
            "usage: key <name>  (e.g. 'key a', 'key enter')".to_owned(),
        );
    };
    match lookup_key(name) {
        Some(spec) => Command::Key(spec),
        None => Command::Invalid(format!("unknown key '{name}'")),
    }
}

fn parse_mouse(args: &[&str]) -> Command {
    let [dx_s, dy_s] = args else {
        return Command::Invalid(
            "usage: mouse <dx> <dy>  (each -127..127)".to_owned(),
        );
    };
    let Ok(dx) = i8::from_str(dx_s) else {
        return Command::Invalid(format!("dx '{dx_s}' is not in -127..127"));
    };
    let Ok(dy) = i8::from_str(dy_s) else {
        return Command::Invalid(format!("dy '{dy_s}' is not in -127..127"));
    };
    Command::Mouse { dx, dy }
}

fn parse_click(args: &[&str]) -> Command {
    let button = match args {
        [] => MouseButton::Left,
        [name] => match name.to_ascii_lowercase().as_str() {
            "left" | "l" => MouseButton::Left,
            "right" | "r" => MouseButton::Right,
            "middle" | "m" => MouseButton::Middle,
            other => {
                return Command::Invalid(format!(
                    "unknown mouse button '{other}' (use left|right|middle)"
                ))
            }
        },
        _ => {
            return Command::Invalid(
                "usage: click [left|right|middle]".to_owned(),
            )
        }
    };
    Command::Click(button)
}

fn parse_media(args: &[&str]) -> Command {
    let [name] = args else {
        return Command::Invalid(
            "usage: media <name>  (e.g. 'media playpause', 'media volup')"
                .to_owned(),
        );
    };
    match lookup_media(name) {
        Some(spec) => Command::Media(spec),
        None => Command::Invalid(format!("unknown media key '{name}'")),
    }
}

/// Look up a keyboard key by name. Accepts single letters (a-z, A-Z),
/// single digits (0-9), or any entry in `NAMED_KEYS` (case-insensitive).
fn lookup_key(raw: &str) -> Option<&'static KeySpec> {
    let trimmed = raw.trim();
    if trimmed.len() == 1 {
        let byte = trimmed.as_bytes()[0];
        if byte.is_ascii_alphabetic() {
            let offset = byte.to_ascii_lowercase() - b'a';
            return Some(letter_key(offset));
        }
        if byte.is_ascii_digit() {
            // HID usage order is 1,2,3,4,5,6,7,8,9,0 starting at 0x1e.
            let digit = byte - b'0';
            let offset = if digit == 0 { 9 } else { digit - 1 };
            return Some(digit_key(offset));
        }
    }
    let lowered = trimmed.to_ascii_lowercase();
    NAMED_KEYS.iter().find(|spec| spec.name == lowered)
}

fn lookup_media(raw: &str) -> Option<&'static MediaSpec> {
    let lowered = raw.trim().to_ascii_lowercase();
    NAMED_MEDIA_KEYS.iter().find(|spec| spec.name == lowered)
}

// Static per-letter / per-digit KeySpec tables so parse() can return a
// &'static KeySpec for any printable single-character key.
fn letter_key(offset: u8) -> &'static KeySpec {
    &LETTER_KEYS[offset as usize]
}
fn digit_key(offset: u8) -> &'static KeySpec {
    &DIGIT_KEYS[offset as usize]
}

macro_rules! letter_spec {
    ($ch:literal, $offset:expr) => {
        KeySpec {
            name: $ch,
            modifier: 0,
            keycode: HID_KEY_A + $offset,
        }
    };
}
macro_rules! digit_spec {
    ($ch:literal, $offset:expr) => {
        KeySpec {
            name: $ch,
            modifier: 0,
            keycode: HID_KEY_1 + $offset,
        }
    };
}

static LETTER_KEYS: [KeySpec; 26] = [
    letter_spec!("a", 0),  letter_spec!("b", 1),  letter_spec!("c", 2),
    letter_spec!("d", 3),  letter_spec!("e", 4),  letter_spec!("f", 5),
    letter_spec!("g", 6),  letter_spec!("h", 7),  letter_spec!("i", 8),
    letter_spec!("j", 9),  letter_spec!("k", 10), letter_spec!("l", 11),
    letter_spec!("m", 12), letter_spec!("n", 13), letter_spec!("o", 14),
    letter_spec!("p", 15), letter_spec!("q", 16), letter_spec!("r", 17),
    letter_spec!("s", 18), letter_spec!("t", 19), letter_spec!("u", 20),
    letter_spec!("v", 21), letter_spec!("w", 22), letter_spec!("x", 23),
    letter_spec!("y", 24), letter_spec!("z", 25),
];

// HID ordering: 1..9 then 0 at the end, so this table is indexed by
// (digit == 0 ? 9 : digit - 1) and stores usage codes 0x1e..0x27.
static DIGIT_KEYS: [KeySpec; 10] = [
    digit_spec!("1", 0), digit_spec!("2", 1), digit_spec!("3", 2),
    digit_spec!("4", 3), digit_spec!("5", 4), digit_spec!("6", 5),
    digit_spec!("7", 6), digit_spec!("8", 7), digit_spec!("9", 8),
    digit_spec!("0", 9),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_key_letter() {
        match parse("key a") {
            Command::Key(spec) => assert_eq!(spec.keycode, HID_KEY_A),
            other => panic!("expected Key, got {other:?}"),
        }
    }

    #[test]
    fn parses_short_key_alias() {
        match parse("k z") {
            Command::Key(spec) => assert_eq!(spec.keycode, HID_KEY_A + 25),
            other => panic!("expected Key, got {other:?}"),
        }
    }

    #[test]
    fn parses_named_key() {
        match parse("key enter") {
            Command::Key(spec) => assert_eq!(spec.keycode, HID_KEY_ENTER),
            other => panic!("expected Key, got {other:?}"),
        }
    }

    #[test]
    fn parses_digit_key() {
        match parse("k 0") {
            Command::Key(spec) => assert_eq!(spec.keycode, 0x27),
            other => panic!("expected Key, got {other:?}"),
        }
        match parse("k 1") {
            Command::Key(spec) => assert_eq!(spec.keycode, 0x1e),
            other => panic!("expected Key, got {other:?}"),
        }
    }

    #[test]
    fn parses_mouse() {
        assert_eq!(parse("mouse 10 -20"), Command::Mouse { dx: 10, dy: -20 });
        assert_eq!(parse("m -5 5"), Command::Mouse { dx: -5, dy: 5 });
    }

    #[test]
    fn rejects_mouse_out_of_range() {
        assert!(matches!(parse("mouse 200 0"), Command::Invalid(_)));
    }

    #[test]
    fn parses_click_default_and_explicit() {
        assert_eq!(parse("click"), Command::Click(MouseButton::Left));
        assert_eq!(parse("c right"), Command::Click(MouseButton::Right));
    }

    #[test]
    fn parses_media() {
        match parse("media playpause") {
            Command::Media(spec) => assert_eq!(spec.usage_code, HID_CONSUMER_PLAY_PAUSE),
            other => panic!("expected Media, got {other:?}"),
        }
        match parse("md volup") {
            Command::Media(spec) => assert_eq!(spec.usage_code, HID_CONSUMER_VOLUME_INCREMENT),
            other => panic!("expected Media, got {other:?}"),
        }
    }

    #[test]
    fn empty_line_is_invalid() {
        assert!(matches!(parse(""), Command::Invalid(_)));
    }

    #[test]
    fn unknown_verb_is_invalid() {
        assert!(matches!(parse("dance"), Command::Invalid(_)));
    }

    #[test]
    fn help_works() {
        assert_eq!(parse("help"), Command::Help);
        assert_eq!(parse("?"), Command::Help);
    }
}

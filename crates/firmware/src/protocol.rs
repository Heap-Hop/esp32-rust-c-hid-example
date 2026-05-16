//! Binary UDP wire format for the HID bridge.
//!
//! Every datagram starts with a 3-byte header:
//!   byte 0  magic   0x48 ('H')   — quick filter for stray traffic
//!   byte 1  version 0x01         — protocol version (this file = v1)
//!   byte 2  opcode               — see `Opcode` below
//! followed by a fixed-size payload determined by the opcode.
//!
//! Replies use a parallel format with the lowercase magic 0x68 ('h') so a
//! client can tell its own datagrams apart from server-originated ones if it
//! happens to share the socket. The only opcode the firmware currently
//! replies to is `PING` — every other command is fire-and-forget.
//!
//! ```text
//! op    name           payload                          notes
//! 0x01  KEY_TAP        u8 modifier, u8 keycode          press + small hold + release
//! 0x02  KEY_DOWN       u8 modifier, u8 keycode          press, no release
//! 0x03  KEY_UP         (none)                           release whatever is held
//! 0x10  MOUSE_MOVE     i8 dx, i8 dy, i8 wheel           relative; wheel may be ignored
//! 0x11  MOUSE_CLICK    u8 button_mask                   down + small hold + up
//! 0x12  MOUSE_BUTTONS  u8 button_mask                   raw button state (for drags)
//! 0x20  MEDIA_TAP      u16 le usage_code                consumer-control key tap
//! 0xf0  PING           u32 le seq                       app → fw, reply is PONG
//! 0xf1  PONG           u32 le seq                       fw → app, echoes seq
//! ```
//!
//! Modifier byte and keycodes use HID Usage Page 0x07; button mask uses the
//! standard HID Mouse bits (bit 0 = left, bit 1 = right, bit 2 = middle);
//! media usage codes are on Usage Page 0x0c.

pub const MAGIC_REQUEST: u8 = b'H';
pub const MAGIC_REPLY: u8 = b'h';
pub const VERSION: u8 = 1;

pub const OP_KEY_TAP: u8 = 0x01;
pub const OP_KEY_DOWN: u8 = 0x02;
pub const OP_KEY_UP: u8 = 0x03;
pub const OP_MOUSE_MOVE: u8 = 0x10;
pub const OP_MOUSE_CLICK: u8 = 0x11;
pub const OP_MOUSE_BUTTONS: u8 = 0x12;
pub const OP_MEDIA_TAP: u8 = 0x20;
pub const OP_PING: u8 = 0xf0;
pub const OP_PONG: u8 = 0xf1;

/// Maximum reply length (`PONG` = 3-byte header + 4-byte seq).
pub const MAX_REPLY_LEN: usize = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    KeyTap { modifier: u8, keycode: u8 },
    KeyDown { modifier: u8, keycode: u8 },
    KeyUp,
    MouseMove { dx: i8, dy: i8, wheel: i8 },
    MouseClick { buttons: u8 },
    MouseButtons { buttons: u8 },
    MediaTap { usage: u16 },
    Ping { seq: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// Datagram is shorter than the 3-byte header.
    TooShort,
    /// Magic byte doesn't match — not one of our packets.
    BadMagic(u8),
    /// Protocol version we don't speak.
    BadVersion(u8),
    /// Opcode is not in the table.
    UnknownOpcode(u8),
    /// Header is OK but the payload is shorter than the opcode requires.
    TruncatedPayload { op: u8, need: usize, got: usize },
}

pub fn parse(buf: &[u8]) -> Result<Command, ParseError> {
    if buf.len() < 3 {
        return Err(ParseError::TooShort);
    }
    if buf[0] != MAGIC_REQUEST {
        return Err(ParseError::BadMagic(buf[0]));
    }
    if buf[1] != VERSION {
        return Err(ParseError::BadVersion(buf[1]));
    }
    let op = buf[2];
    let payload = &buf[3..];

    match op {
        OP_KEY_TAP => take(op, payload, 2).map(|p| Command::KeyTap {
            modifier: p[0],
            keycode: p[1],
        }),
        OP_KEY_DOWN => take(op, payload, 2).map(|p| Command::KeyDown {
            modifier: p[0],
            keycode: p[1],
        }),
        OP_KEY_UP => Ok(Command::KeyUp),
        OP_MOUSE_MOVE => take(op, payload, 3).map(|p| Command::MouseMove {
            dx: p[0] as i8,
            dy: p[1] as i8,
            wheel: p[2] as i8,
        }),
        OP_MOUSE_CLICK => take(op, payload, 1).map(|p| Command::MouseClick { buttons: p[0] }),
        OP_MOUSE_BUTTONS => {
            take(op, payload, 1).map(|p| Command::MouseButtons { buttons: p[0] })
        }
        OP_MEDIA_TAP => take(op, payload, 2).map(|p| Command::MediaTap {
            usage: u16::from_le_bytes([p[0], p[1]]),
        }),
        OP_PING => take(op, payload, 4).map(|p| Command::Ping {
            seq: u32::from_le_bytes([p[0], p[1], p[2], p[3]]),
        }),
        other => Err(ParseError::UnknownOpcode(other)),
    }
}

fn take(op: u8, payload: &[u8], n: usize) -> Result<&[u8], ParseError> {
    if payload.len() < n {
        Err(ParseError::TruncatedPayload {
            op,
            need: n,
            got: payload.len(),
        })
    } else {
        Ok(&payload[..n])
    }
}

/// Write a PONG into `out` and return the number of bytes used.
pub fn write_pong(seq: u32, out: &mut [u8; MAX_REPLY_LEN]) -> usize {
    out[0] = MAGIC_REPLY;
    out[1] = VERSION;
    out[2] = OP_PONG;
    out[3..7].copy_from_slice(&seq.to_le_bytes());
    7
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkt(op: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![MAGIC_REQUEST, VERSION, op];
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn rejects_short() {
        assert_eq!(parse(&[]), Err(ParseError::TooShort));
        assert_eq!(parse(&[0x48, 0x01]), Err(ParseError::TooShort));
    }

    #[test]
    fn rejects_bad_magic() {
        assert_eq!(parse(&[0x00, 0x01, 0x01]), Err(ParseError::BadMagic(0x00)));
    }

    #[test]
    fn rejects_bad_version() {
        assert_eq!(
            parse(&[MAGIC_REQUEST, 0x99, 0x01]),
            Err(ParseError::BadVersion(0x99))
        );
    }

    #[test]
    fn parses_key_tap() {
        let buf = pkt(OP_KEY_TAP, &[0x02, 0x04]);
        assert_eq!(
            parse(&buf),
            Ok(Command::KeyTap {
                modifier: 0x02,
                keycode: 0x04
            })
        );
    }

    #[test]
    fn parses_key_up_without_payload() {
        let buf = pkt(OP_KEY_UP, &[]);
        assert_eq!(parse(&buf), Ok(Command::KeyUp));
    }

    #[test]
    fn parses_mouse_move_signed() {
        let buf = pkt(OP_MOUSE_MOVE, &[0xfe, 0x05, 0x00]); // dx=-2, dy=5, wheel=0
        assert_eq!(
            parse(&buf),
            Ok(Command::MouseMove {
                dx: -2,
                dy: 5,
                wheel: 0,
            })
        );
    }

    #[test]
    fn parses_media_tap_little_endian() {
        let buf = pkt(OP_MEDIA_TAP, &[0xcd, 0x00]); // play/pause = 0x00cd
        assert_eq!(parse(&buf), Ok(Command::MediaTap { usage: 0x00cd }));
    }

    #[test]
    fn parses_ping_and_writes_matching_pong() {
        let buf = pkt(OP_PING, &[0x39, 0x05, 0x00, 0x00]);
        assert_eq!(parse(&buf), Ok(Command::Ping { seq: 0x0539 }));

        let mut out = [0u8; MAX_REPLY_LEN];
        let n = write_pong(0x0539, &mut out);
        assert_eq!(n, 7);
        assert_eq!(out, [MAGIC_REPLY, VERSION, OP_PONG, 0x39, 0x05, 0x00, 0x00]);
    }

    #[test]
    fn rejects_truncated_payload() {
        let buf = pkt(OP_MOUSE_MOVE, &[0x01]); // expected 3 bytes
        assert_eq!(
            parse(&buf),
            Err(ParseError::TruncatedPayload {
                op: OP_MOUSE_MOVE,
                need: 3,
                got: 1,
            })
        );
    }

    #[test]
    fn rejects_unknown_opcode() {
        let buf = pkt(0x7f, &[]);
        assert_eq!(parse(&buf), Err(ParseError::UnknownOpcode(0x7f)));
    }
}

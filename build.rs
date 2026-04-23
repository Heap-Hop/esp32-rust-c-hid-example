fn main() {
    // Emit the linker flags and environment variables that esp-idf-sys needs.
    // This single call is the only required content for most projects.
    embuild::espidf::sysenv::output();
}

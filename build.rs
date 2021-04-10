//! Our build script.
//!
//! Currently, this just makes certain compile-time information visible to
//! the application using built.

fn main() {
    built::write_built_file().expect("Failed to acquire build-time information")
}

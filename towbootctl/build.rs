//! Our build script.
//!
//! It makes certain compile-time information visible to the application using built.
use std::env;

fn main() {
    if env::var("CARGO_FEATURE_BINARY").is_ok() {
        built::write_built_file().expect("Failed to acquire build-time information");
    }
}

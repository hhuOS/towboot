//! This is a dummy crate that contains a towboot binary for i686.
//! 
//! This is needed because <https://github.com/rust-lang/cargo/pull/10061>
//! is not ready yet.

/// The towboot binary for i686.
pub const TOWBOOT: &[u8] = include_bytes!(env!("CARGO_BIN_FILE_TOWBOOT_towboot"));

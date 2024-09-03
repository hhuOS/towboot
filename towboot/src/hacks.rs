//! "This place is not a place of honor.
//! No highly esteemed deed is commemorated here.
//! Nothing valued is here."
//!
//! This module contains missing symbols.
//! 
//! The fmod and fmodf functions are [currently missing](https://github.com/rust-lang/rust/issues/128533)
//! on i686-unknown-uefi, so let's use the ones of libm.
//! In the long run, this should be fixed in `compiler_builtins`.
//! For now, this monkeypatching seems to be enough.
//!
//! see https://github.com/rust-lang/compiler-builtins/blob/master/src/math.rs
#[cfg(target_arch = "x86")]
#[no_mangle]
pub extern "C" fn fmod(x: f64, y: f64) -> f64 {
    libm::fmod(x, y)
}
#[cfg(target_arch = "x86")]
#[no_mangle]
pub extern "C" fn fmodf(x: f32, y: f32) -> f32 {
    libm::fmodf(x, y)
}

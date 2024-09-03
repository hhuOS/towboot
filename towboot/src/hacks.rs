//! "This place is not a place of honor.
//! No highly esteemed deed is commemorated here.
//! Nothing valued is here."
//!
//! This module contains missing symbols.
//! 
//! The fmod and fmodf functions are [currently missing](https://github.com/rust-lang/rust/issues/128533)
//! on i686-unknown-uefi. In the long run, this should be fixed in
//! `compiler_builtins`. For now, this monkeypatching seems to be enough.
//!
//! see https://github.com/rust-lang/compiler-builtins/blob/master/src/math.rs
//! We could also use libm::fmod{,f} here, but then we'd need __truncdfsf2.
//! Let's just hope they are never called.
#[cfg(target_arch = "x86")]
#[no_mangle]
pub extern "C" fn fmod(_x: f64, _y: f64) -> f64 {
    unimplemented!();
}
#[cfg(target_arch = "x86")]
#[no_mangle]
pub extern "C" fn fmodf(_x: f32, _y: f32) -> f32 {
    unimplemented!();
}

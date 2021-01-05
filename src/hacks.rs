//! "This place is not a place of honor.
//! No highly esteemed deed is commemorated here.
//! Nothing valued is here."
//!
//! This module contains a few hacks to make stuff work in this environment.
//! The UEFI targets are only Tier 3 which means they are rarely tested and may not work fully.
//! (see <https://doc.rust-lang.org/nightly/rustc/platform-support.html#tier-3>)
//! The code below is mostly adding unknown symbols.
//! In the long run, they should be reported to compiler_builtins and fixed there.
//! For now, this monkeypatching seems to be enough.

// fmod and fmodf seem to not be supported (yet) by compiler_builtins for uefi
// see https://github.com/rust-lang/compiler-builtins/blob/master/src/math.rs
// We could use libm::fmod{,f} here, but then we'd need __truncdfsf2.
// This once was in compiler_builtins, but it's not anymore.
// see https://github.com/rust-lang/compiler-builtins/pull/262
// So, let's just hope they are never called.
#[no_mangle]
pub extern "C" fn fmod(_x: f64, _y: f64) -> f64 {
    unimplemented!();
}
#[no_mangle]
pub extern "C" fn fmodf(_x: f32, _y: f32) -> f32 {
    unimplemented!();
}

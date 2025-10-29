#![no_std]
#![no_main]

use core::panic::PanicInfo;

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
pub enum CoolStruct {
}

#[unsafe(no_mangle)]
fn efi_main() {
    loop {}
}

#[panic_handler]
fn panic_handler(_info: &PanicInfo) -> ! {
    loop {}
}

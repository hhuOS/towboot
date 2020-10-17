#![no_std]
#![no_main]
#![feature(abi_efiapi)]
#![feature(global_asm)]

extern crate rlibc;

use core::fmt::Write;

use uefi::prelude::*;
use uefi::proto::loaded_image::LoadedImage;
use uefi::proto::media::fs::SimpleFileSystem;

#[entry]
fn efi_main(image: Handle, systab: SystemTable<Boot>) -> Status {
    uefi_services::init(&systab).expect_success("Failed to initialize utilities");
    writeln!(systab.stdout(), "Hello, world!").unwrap();
    
    // get information about the way we were loaded
    // the interesting thing here is the partition handle
    let loaded_image = systab.boot_services()
    .handle_protocol::<LoadedImage>(image)
    .expect_success("Failed to open loaded image protocol");
    let loaded_image = unsafe { &mut *loaded_image.get() };
    
    // open the filesystem
    let fs = systab.boot_services()
    .handle_protocol::<SimpleFileSystem>(loaded_image.device())
    .expect_success("Failed to open filesystem");
    let fs = unsafe { &mut *fs.get() };
    let volume = fs.open_volume().expect_success("Failed to open root directory");
    
    Status::SUCCESS
}

// this is a bug in Rust's compiler-builtins/src/probestack.rs
// Noone seems to be using i686-unknown-uefi.
global_asm!("
.globl ___rust_probestack
___rust_probestack:
    jmp __rust_probestack
");


#![no_std]
#![no_main]
#![feature(abi_efiapi)]
#![feature(global_asm)]

extern crate rlibc;

use core::fmt::Write;

use uefi::prelude::*;

#[entry]
fn efi_main(image: Handle, systab: SystemTable<Boot>) -> Status {
    uefi_services::init(&systab).expect_success("Failed to initialize utilities");
    writeln!(systab.stdout(), "Hello, world!").unwrap();
    Status::SUCCESS
}

// this is a bug in Rust's compiler-builtins/src/probestack.rs
// Noone seems to be using i686-unknown-uefi.
global_asm!("
.globl ___rust_probestack
___rust_probestack:
    jmp __rust_probestack
");


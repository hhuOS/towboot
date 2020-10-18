# bootloader

a bootloader for Multiboot kernels on UEFI systems

## build dependencies

You'll need a nightly Rust compiler.
The version doesn't really matter, though 2020-10-05 definitely works.
If you don't know how to install one,
please take a look at [rustup.rs](https://rustup.rs/).

To build a disk image, you'll also need mtools and mkgpt.
The latter one is automatically being downloaded and compiled,
you'll need git, automake, make and a C compiler for that.

To boot the disk image in a virtual machine, QEMU is recommended.

## building

`cargo build` creates a `bootloader.efi` file inside the `target` folder.

But running `./build.sh` will do that and also create a disk image
and boot that with QEMU, so just may just want to run this.

## documentation

This README file is relatively short (as you can see).
More documentation is available by running `cargo doc --open`.

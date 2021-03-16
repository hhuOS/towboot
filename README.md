# towboot

a bootloader for Multiboot kernels on UEFI systems

## build dependencies

You'll need a nightly Rust compiler.
The version doesn't really matter,
though rustc 1.51.0-nightly (61f5a0092 2021-01-04) definitely works.
If you don't know how to install one,
please take a look at [rustup.rs](https://rustup.rs/).
(You can configure rustup to use a nightly toolchain just for the current folder
by running `rustup override set nightly`.)

To build a disk image, you'll also need mtools and mkgpt.
The latter one is automatically being downloaded and compiled,
you'll need git, automake, make and a C compiler for that.

To boot the disk image in a virtual machine, QEMU is recommended.
You'll need OVMF for that, too. You can either install it via your distribution's
package manager or (for `i686`) let the build script download it.

## building

`cargo build` creates a `towboot.efi` file inside the `target` folder.
By default, this is a debug build for `i686-unknown-uefi`.
You can change this by appending `--release`
or by setting `--target x86_64_unknown_uefi` (for example).

Running `./build.sh` will do that and also create a disk image
and boot that with QEMU, so just may just want to run this.

You can configure whether to create a `debug` or `release` build for
either `i686` or `x86_64`, whether to enable KVM or wait for a GDB to attach
by setting the environment variables `BUILD`, `ARCH`, `KVM` or `GDB`.
(The defaults are `debug`, `i686`, `no` and `no`.)

This script expects the kernel in `../../kernels/multiboot1.elf`,
you can override this by setting `KERNEL`.

## documentation

This README file is relatively short (as you can see).
More documentation is available by running `cargo doc --open`.

## Known bugs / workarounds

The `hacks` modules contains workarounds for bugs or missing features in
the compiler.

The function `mem::Allocation::new_under_4gb` is modified to keep allocations
below 200MB. This may break for many or big modules or kernels, but seems to
be needed for other kernels. The value might need to be adjusted or turned into
a runtime flag.

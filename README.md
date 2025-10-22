# towboot

a bootloader for Multiboot kernels (version 1 and version 2) on UEFI systems

## usage

towboot is a UEFI application. If you're developing an operating system or just
want to create a boot medium, there are several possible options for where to
place towboot and its configuration:

### removable media

This is the easiest one: It works for all architectures and requires no
configuration of the system.
Simply place the 32-bit build at `\EFI\boot\bootia32.efi`, the 64-bit build at
`\EFI\boot\bootx64.efi` and a configuration file at `\towboot.toml` on the ESP.

You can also use the provided `towbootctl` binary to do this.

```sh
towbootctl install <path_to_the_esp> --removable -- -config towboot.toml
```

This will parse the configuration file and copy the configuration itself,
the referenced kernels and modules and towboot binaries for 32-bit and 64-bit
to the target directory.

### installed system

Place an appropriate build at `\EFI\yourOS\towboot.efi` and the configuration
at `\EFI\yourOS\towboot.toml` on the ESP and add a boot option for
`\EFI\yourOS\towboot.efi -c \EFI\yourOS\towboot.toml`.

towbootctl can help you a bit with this:

```sh
towbootctl install <path_to_the_esp> --name yourOS -- -config towboot.toml
```

(You can also configure towboot just with command line arguments instead of
using a configuration file; see below.)

### image

If you're not installing to physical media but instead want to create an image,
towbootctl can do this as well:

```sh
towbootctl image --target yourOS.img -- -config towboot.toml
towbootctl boot-image --image yourOS.img
```

### chainloading from another bootloader

If you already have a bootloader capable of loading UEFI applications but
without support for Multiboot, you can add an entry like
`towboot.efi -kernel "mykernel.elf quiet" -module "initramfs.img initrd"`.

(You can use a configuration file instead of passing the information directly
on the command line; see above.)

### paths

Paths given in a configuration file or on the command line are interpreted as
follows:
 * absolute if they start with a volume identifier (`fs?:`)
 * relative to the volume towboot itself is on if they start with a backslash (`\`)
 * relative to the configuration file
 * relative to the UEFI shell's current working directory

Paths for kernel and modules given on the commandline can't contain spaces,
use a configuration file for this.

### quirks

You can override some specifics of how the kernel is loaded at runtime by
adding quirks. They can be configured either in the `quirk` key of a kernel
entry (if the kernel is loaded via a configuration file) or via the `-quirk`
command line option (if the kernel is loaded via `-kernel`).

Available quirks are:

* `DontExitBootServices`: do not exit Boot Services
        This starts the kernel with more privileges and less available memory.
        In some cases this might also display more helpful error messages.
* `ForceElf`: always treat the kernel as an ELF file
* `ForceOverwrite`: ignore the memory map when loading the kernel
        (This might damage your hardware!)
* `KeepResolution`: ignore the kernel's preferred resolution
* `ModulesBelow200Mb`: keep allocations for modules below 200 MB

## development

If you want to compile towboot yourself, here are the instructions:

### dependencies

You'll need a nightly Rust compiler.
The version doesn't really matter,
though `1.88.0-nightly (6bc57c6bf 2025-04-22)` definitely works.
If you don't know how to install one,
please take a look at [rustup.rs](https://rustup.rs/).

To boot the disk image in a virtual machine, QEMU is recommended.
You'll need OVMF for that, too, but the build script downloads it by itself.

### building

```sh
cargo build --package towboot
```

creates a `towboot.efi` file inside the `target` folder.
By default, this is a debug build for `i686-unknown-uefi`.
You can change this by appending `--release`
or by setting `--target x86_64_unknown_uefi` (for example).

Running `cargo xtask build` will do that and also create a disk image,
so just may just want to run this. To boot the resulting image with QEMU,
you can use `cargo xtask boot-image`.

You can configure whether to create a `debug` or `release` build for
either `i686` or `x86_64`, whether to enable KVM or wait for a GDB to attach
by specifying command line options.

You can also run towbootctl directly from the source directory (building it will
also build towboot, in turn):

```sh
cargo run --package towbootctl --features binary
```

### running the tests

The integration tests can be run with:

```sh
cargo test --package tests
```

## project structure

This project is a Cargo workspace consisting of the multiple packages.
More documentation for each of them is available by running:

```sh
cargo doc --package <package> --open
```

### towboot

This is the actual bootloader.

### towboot_config

This is a library containing the configuration structs.
It is used by towboot and towbootctl.

### towboot_ia32 / towboot_x64

These are dummy crates that just exists to provide the towboot binary in library form.

### towbootctl

This is both a library and a command line utility that can create images,
install towboot to disk, and so on.

### tests

This contains the integration tests.

### xtask

This contains build tooling.

## contributing

This project follows the usual GitHub workflow consisting of fork, pull request
and merge. If you don't have a GitHub account, you can also send patches per
e-mail.

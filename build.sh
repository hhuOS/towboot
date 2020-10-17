#!/bin/sh
set -eu
cargo build
cargo build --target x86_64-unknown-uefi

mkdir -p image

dd if=/dev/zero of=image.img count=50 bs=1M
parted image.img mktable gpt
echo ignore | parted image.img mkpart fat32 1 100%
LOOP=$(sudo losetup --partscan --find image.img --show)
sudo mkfs.vfat ${LOOP}p1
sudo mount ${LOOP}p1 image/
sudo mkdir -p image/efi/boot
sudo cp target/i686-unknown-uefi/debug/bootloader.efi image/efi/boot/bootia32.efi
sudo cp target/x86_64-unknown-uefi/debug/bootloader.efi image/efi/boot/bootx64.efi
sudo umount image/
sudo losetup -d $LOOP

qemu-system-i386 -machine pc,accel=kvm,kernel-irqchip=off -bios ~/bin/OVMF.bin -hda image.img

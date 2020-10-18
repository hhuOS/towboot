#!/bin/sh
set -eu
cargo build
# cargo build --target x86_64-unknown-uefi

echo "checking whether mkgpt exists and building it if not…"
if [ ! -d mkgpt ]
then
    git clone https://github.com/jncronin/mkgpt.git
fi
if [ ! -f mkgpt/mkgpt ]
then
    cd mkgpt
    aclocal
    automake --add-missing
    ./configure
    make
    cd ..
fi

echo "building image…"
mformat -i part.img -C -F -T $(echo "100 * 1024" | bc) -h 1 -s 1024 :: # 50 MiB
mmd -i part.img efi
mmd -i part.img efi/boot
mcopy -i part.img target/i686-unknown-uefi/debug/bootloader.efi ::efi/boot/bootia32.efi
mcopy -i part.img target/x86_64-unknown-uefi/debug/bootloader.efi ::efi/boot/bootx64.efi
mcopy -i part.img bootloader.toml ::
mcopy -i part.img ../../kernels/multiboot1.elf ::
mcopy -i part.img ~/dev/hhuOS/loader/boot/hhuOS.bin ::
mcopy -i part.img ~/dev/hhuOS/loader/boot/hhuOS.initrd ::

mkgpt/mkgpt -o image.img --part part.img --type system
rm part.img

echo "launching qemu…"
qemu-system-i386 -machine pc,accel=kvm,kernel-irqchip=off -bios ~/bin/OVMF.bin -hda image.img
# qemu-system-x86_64 -machine pc,accel=kvm,kernel-irqchip=off -bios /usr/share/ovmf/OVMF.fd -hda image.img

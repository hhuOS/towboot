#!/bin/sh
set -eu

BUILD=${BUILD:-debug}
if [ $BUILD = "release" ]
then
    BUILD_FLAGS="--release"
else
    BUILD_FLAGS=""
fi

ARCH=${ARCH:-i686} # or x86_64
if [ $ARCH = "i686" ]
then
    EFIARCH="ia32"
    QEMUARCH="i386"
    OVMF_PATH=~/bin/OVMF.bin
elif [ $ARCH = "x86_64" ]
then
    EFIARCH="x64"
    QEMUARCH=$ARCH
    OVMF_PATH="/usr/share/ovmf/OVMF.fd"
else
    echo "unknown arch $ARCH"
    return 1
fi
echo "building $BUILD for $ARCH, set BUILD or ARCH to override…"
cargo build --target $ARCH-unknown-uefi $BUILD_FLAGS

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
mcopy -i part.img target/$ARCH-unknown-uefi/$BUILD/bootloader.efi ::efi/boot/boot$EFIARCH.efi
mcopy -i part.img bootloader.toml ::
mcopy -i part.img ../../kernels/multiboot1.elf ::
mcopy -i part.img ~/dev/hhuOS/loader/boot/hhuOS.bin ::
mcopy -i part.img ~/dev/hhuOS/loader/boot/hhuOS.initrd ::

mkgpt/mkgpt -o image.img --part part.img --type system
rm part.img

echo "launching qemu…"
qemu-system-$QEMUARCH -machine pc,accel=kvm,kernel-irqchip=off -bios $OVMF_PATH -hda image.img

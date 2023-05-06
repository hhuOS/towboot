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
elif [ $ARCH = "x86_64" ]
then
    EFIARCH="x64"
    QEMUARCH=$ARCH
else
    echo "unknown arch $ARCH"
    return 1
fi
KVM=${KVM:-no}
if [ $KVM = "yes" ]
then
    QEMUMACHINE="-machine pc,accel=kvm,kernel-irqchip=off"
elif [ $KVM = "no" ]
then
    QEMUMACHINE=""
else
    echo "KVM has to be either yes or no, but is $KVM"
    return 1
fi
GDB=${GDB:-no}
if [ $GDB = "yes" ]
then
    QEMUDEBUG="-S -s"
elif [ $GDB = "no" ]
then
    QEMUDEBUG=""
else
    echo "GDB has to be either yes or no, but is $GDB"
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
KERNEL=${KERNEL:-../../kernels/multiboot1.elf}
echo "Using $KERNEL, set KERNEL to override."
mformat -i part.img -C -F -T $(echo "100 * 1024" | bc) -h 1 -s 1024 :: # 50 MiB
mmd -i part.img efi
mmd -i part.img efi/boot
mcopy -i part.img target/$ARCH-unknown-uefi/$BUILD/towboot.efi ::efi/boot/boot$EFIARCH.efi
mcopy -i part.img towboot.toml ::
mcopy -i part.img "$KERNEL" ::multiboot.elf

mkgpt/mkgpt -o image.img --part part.img --type system
rm part.img

echo "downloading OVMF if needed"
cd ovmf/ && bash download.sh; cd ..

FIRMWARE=${FIRMWARE:-ovmf/$EFIARCH/OVMF.fd}
echo "Using $FIRMWARE as the firmware, set FIRMWARE to override."

echo "launching qemu with KVM=$KVM…"
if [ $GDB = "yes" ]
then
    echo "The machine starts paused, waiting for GDB to attach to localhost:1234."
fi
qemu-system-$QEMUARCH $QEMUMACHINE $QEMUDEBUG -bios "$FIRMWARE" \
-serial stdio \
-drive driver=raw,node-name=disk,file.driver=file,file.filename=image.img -m 256

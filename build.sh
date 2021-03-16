#!/bin/sh
set -eu

OVMF_i686_RPM_FILE_NAME="edk2-ovmf-ia32-20200801stable-1.fc33.noarch.rpm"
OVMF_i686_RPM_URL="https://download-ib01.fedoraproject.org/pub/fedora/linux/releases/33/Everything/x86_64/os/Packages/e/$OVMF_i686_RPM_FILE_NAME"

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
    OVMF_PATH="OVMF.fd"
elif [ $ARCH = "x86_64" ]
then
    EFIARCH="x64"
    QEMUARCH=$ARCH
    OVMF_PATH="/usr/share/ovmf/OVMF.fd"
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
mcopy -i part.img $KERNEL ::multiboot1.elf

mkgpt/mkgpt -o image.img --part part.img --type system
rm part.img

echo "checking whether OVMF exists, else trying to download…"
if [ ! -f $OVMF_PATH ]
then
    if [ $ARCH = "i686" ]
    then
        wget $OVMF_i686_RPM_URL
        if [ -n $(which rpm2cpio) ]
        then
            rpm2cpio $OVMF_i686_RPM_FILE_NAME > $OVMF_i686_RPM_FILE_NAME.cpio
        elif [ -n $(which unzstd) ]
        then
            7z x $OVMF_i686_RPM_FILE_NAME
            unzstd *.zstd -o $OVMF_i686_RPM_FILE_NAME.cpio # TODO
        else
            echo "You'll need either rpm2cpio or 7z and unzstd (or just drop a file to $OVMF_PATH)."
            return 1
        fi
        rm $OVMF_i686_RPM_FILE_NAME
        cpio --extract --file $OVMF_i686_RPM_FILE_NAME.cpio -d
        rm $OVMF_i686_RPM_FILE_NAME.cpio
        mv usr/share/edk2/ovmf-ia32/OVMF_CODE.fd OVMF.fd
        rm -r usr
    else
        echo "Don't know where to download OVMF for $ARCH to $OVMF_PATH."
        return 1
    fi
fi

echo "launching qemu with KVM=$KVM…"
if [ $GDB = "yes" ]
then
    echo "The machine starts paused, waiting for GDB to attach to localhost:1234."
fi
qemu-system-$QEMUARCH $QEMUMACHINE $QEMUDEBUG -bios $OVMF_PATH \
-serial stdio \
-drive driver=raw,node-name=disk,file.driver=file,file.filename=image.img -m 256

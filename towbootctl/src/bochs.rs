//! This module allows booting with Bochs.
use std::io::Write;
use std::path::Path;

use anyhow::Result;
use tempfile::NamedTempFile;

/// Generate a appropriate bochrs file.
pub fn bochsrc(ovmf: &Path, image: &Path, gdb: bool) -> Result<NamedTempFile> {
    let ovmf = ovmf.display();
    let image = image.display();
    let gdb: u8 = gdb.into();
    let mut file = NamedTempFile::new()?;
    write!(file.as_file_mut(), "
# partly taken from https://forum.osdev.org/viewtopic.php?f=1&t=33440
display_library: x
megs: 768
romimage: file=\"{ovmf}\", address=0x0, options=none
vgaromimage: file=\"/usr/share/bochs/VGABIOS-lgpl-latest\"
ata0: enabled=1, ioaddr1=0x1f0, ioaddr2=0x3f0, irq=14
ata0-master: type=disk, path=\"{image}\", mode=flat, cylinders=0, heads=0, spt=0, sect_size=512, model=\"Generic 1234\", biosdetect=auto, translation=auto
ata0-slave: type=none
pci: enabled=1, chipset=i440fx, slot1=cirrus
vga: extension=cirrus, update_freq=5, realtime=1
print_timestamps: enabled=0
port_e9_hack: enabled=0
private_colormap: enabled=0
clock: sync=none, time0=local, rtc_sync=0
log: -
logprefix: %t%e%d
debug: action=ignore
info: action=report
error: action=report
panic: action=ask
keyboard: type=mf, serial_delay=250, paste_delay=100000, user_shortcut=none
mouse: type=ps2, enabled=0, toggle=ctrl+mbutton
sound: waveoutdrv=win, waveout=none, waveindrv=win, wavein=none, midioutdrv=win, midiout=none
speaker: enabled=1, mode=sound
parport1: enabled=1, file=none
com1: enabled=1, mode=null
gdbstub: enabled={gdb}, port=1234, text_base=0, data_base=0, bss_base=0
")?;
    Ok(file)
}

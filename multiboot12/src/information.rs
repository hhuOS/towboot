use core::alloc::{Allocator, Layout};
use alloc::boxed::Box;
use alloc::vec::Vec;
use alloc::slice;
use alloc::{collections::BTreeMap, alloc::dealloc};
use update_cell::UpdateCell;

use multiboot::information::{
    ColorInfoRgb,
    ColorInfoType,
    ElfSymbols,
    FramebufferTable,
    MemoryManagement,
    Multiboot,
    MultibootInfo,
    MemoryEntry as MultibootMemoryEntry,
    MemoryType as MultibootMemoryType,
    Module as MultibootModule,
    PAddr,
    SymbolType,
    SIGNATURE_EAX as MULTIBOOT_EAX_SIGNATURE,
};
use multiboot2::{
    BasicMemoryInfoTag, BootInformation as Multiboot2BootInformation,    BootLoaderNameTag, CommandLineTag, EFIBootServicesNotExitedTag,
    EFIImageHandle32Tag, EFIImageHandle64Tag, EFIMemoryMapTag, EFISdt32Tag,
    EFISdt64Tag, ElfSectionsTag, FramebufferField, FramebufferTag,
    FramebufferType, ImageLoadPhysAddrTag, MAGIC as MULTIBOOT2_EAX_SIGNATURE,
    MemoryArea, MemoryAreaType, MemoryMapTag, ModuleTag, RsdpV1Tag, RsdpV2Tag,
    SmbiosTag,
};
pub use multiboot2::EFIMemoryDesc as EfiMemoryDescriptor;
use multiboot2::Builder as Multiboot2InformationBuilder;
use ouroboros::self_referencing;

pub type MemoryUpdateFunction = Box<dyn FnMut(&mut [u8], u32, u32, &[MemoryEntry], Option<&[EfiMemoryDescriptor]>)>;

pub enum InfoBuilder<A: Allocator + 'static> {
    Multiboot(MultibootInfoBuilder<A>),
    Multiboot2(UpdateCell<Multiboot2InformationBuilder>),
}

impl<A: Allocator + Clone + 'static> InfoBuilder<A> {
    pub fn new_multiboot(allocator: A) -> Self {
        Self::Multiboot(MultibootInfoBuilder::new(
            MultibootInfo::default(), MultibootAllocator::new(allocator.clone()),
            Vec::new_in(allocator), |i, a| Multiboot::from_ref(i, a),
        ))
    }

    #[must_use]
    pub fn new_multiboot2() -> Self {
        Self::Multiboot2(UpdateCell::new(Multiboot2InformationBuilder::new()))
    }

    /// Note: This allocates.
    /// Also, since the return value contains a Box, dropping it deallocates.
    pub fn build(self, allocator: A) -> (Vec<u8, A>, u32, MemoryUpdateFunction) {
        match self {
            Self::Multiboot(bu) => {
                let mut heads = bu.into_heads();
                (
                    unsafe { core::slice::from_raw_parts(
                        (&raw const heads.info).cast::<u8>(),
                        core::mem::size_of::<MultibootInfo>(),
                    ) }.to_vec_in(allocator),
                    MULTIBOOT_EAX_SIGNATURE,
                    Box::new(move |info_bytes: &mut [u8], lower: u32, upper: u32, entries: &[MemoryEntry], _efi_mmap: Option<&[EfiMemoryDescriptor]>| {
                        let (_head, body, _tail) = unsafe {
                            info_bytes.align_to_mut::<MultibootInfo>()
                        };
                        let info = &mut body[0];
                        let mut multiboot = Multiboot::from_ref(
                            info, &mut heads.allocator,
                        );
                        multiboot.set_memory_bounds(Some((lower, upper)));
                        MultibootInfoBuilder::<A>::copy_memory_regions(
                            &mut heads.memory_map_vec, entries,
                        );
                        multiboot.set_memory_regions(Some((heads.memory_map_vec.as_slice().as_ptr() as PAddr, entries.len())));
                    }),
                )
            },
            Self::Multiboot2(c) => {
                let header = c.into_inner().build();
                let len: usize = header.header().total_size().try_into().unwrap();
                (
                    {
                        let v = unsafe { Vec::from_raw_parts(Box::into_raw(header).cast(), len, len) };
                        // this copies the Vec to the given allocator
                        v.to_vec_in(allocator)
                    },
                    MULTIBOOT2_EAX_SIGNATURE,
                    Box::new(|info_bytes: &mut [u8], lower: u32, upper: u32, entries: &[MemoryEntry], efi_mmap: Option<&[EfiMemoryDescriptor]>| {
                        let info = unsafe {
                            Multiboot2BootInformation::load(info_bytes.as_ptr().cast())
                        }.unwrap();
                        let mem_map_tag = info.memory_map_tag().unwrap();
                        entries.iter().zip(
                            mem_map_tag.memory_areas()
                        ).for_each(
                            |(source, destination)| match source {
                                MemoryEntry::Multiboot(_)
                                    => panic!("wrong Multiboot version"),
                                MemoryEntry::Multiboot2(src) => {
                                    let destination = core::ptr::from_ref::<MemoryArea>(destination).cast_mut();
                                    unsafe { destination.write(*src) };
                                },
                            }
                        );
                        let mem_info_tag = info.basic_memory_info_tag().unwrap();
                        let mem_info_tag = core::ptr::from_ref::<BasicMemoryInfoTag>(mem_info_tag).cast_mut();
                        unsafe { mem_info_tag.write(BasicMemoryInfoTag::new(lower, upper)) };
                        if let Some(mmap) = efi_mmap {
                            // we can't get the EFIMemoryMapTag if there is a BootServicesNotExitedTag
                            if let Some(efi_mmap_tag) = info.efi_memory_map_tag() {
                                mmap.iter().zip(
                                    efi_mmap_tag.memory_areas()
                                ).for_each(|(src, dest)| {
                                    let dest = core::ptr::from_ref::<EfiMemoryDescriptor>(dest).cast_mut();
                                    unsafe { dest.write(*src) };
                                });
                            }
                        }
                    }),
                )
            },
        }
    }

    pub const fn new_color_info_rgb(&self,
        red_field_position: u8,
        red_mask_size: u8,
        green_field_position: u8,
        green_mask_size: u8,
        blue_field_position: u8,
        blue_mask_size: u8,
    ) -> ColorInfo {
        match self {
            Self::Multiboot(_) => ColorInfo::Multiboot(ColorInfoType::Rgb(ColorInfoRgb {
                red_field_position,
                red_mask_size,
                green_field_position,
                green_mask_size,
                blue_field_position,
                blue_mask_size,
            })),
            Self::Multiboot2(_) => ColorInfo::Multiboot2(FramebufferType::RGB {
                red: FramebufferField {
                    position: red_field_position,
                    size: red_mask_size,
                },
                green: FramebufferField {
                    position: green_field_position,
                    size: green_mask_size,
                },
                blue: FramebufferField {
                    position: blue_field_position,
                    size: blue_mask_size,
                },
            }),
        }
    }

    pub fn new_memory_entry(&self, base_addr: u64, length: u64, ty: MemoryType, ) -> MemoryEntry {
        match self {
            Self::Multiboot(_) => MemoryEntry::Multiboot(
                MultibootMemoryEntry::new(base_addr, length, MultibootMemoryType::from(ty))
            ),
            Self::Multiboot2(_) => MemoryEntry::Multiboot2(
                MemoryArea::new(base_addr, length, MemoryAreaType::from(ty))
            ),
        }
    }

    pub fn allocate_memory_map_vec(&mut self, count: usize) -> Vec<MemoryEntry> {
        match self {
            Self::Multiboot(b) => b.allocate_memory_map_vec(count),
            Self::Multiboot2(c) => {
                // allocate empty memory entries
                let mut v = Vec::new();
                v.resize_with(count, || MemoryArea::new(0, 0, MemoryAreaType::Reserved));
                c.update(|b| b.mmap(
                    MemoryMapTag::new(v.as_slice())
                ));
            },
        }
        let mut v = Vec::new();
        v.resize_with(
            count, || self.new_memory_entry(0, 0, MemoryType::Reserved),
        );
        v
    }

    pub fn allocate_efi_memory_map_vec(&mut self, count: usize) -> Vec<EfiMemoryDescriptor> {
        match self {
            // Multiboot1 doesn't support passing EFI memory maps.
            Self::Multiboot(_) => (),
            Self::Multiboot2(c) => {
                // allocate empty memory entries
                let mut v = Vec::new();
                v.resize(count, EfiMemoryDescriptor::default());
                c.update(|b| b.efi_mmap(
                    EFIMemoryMapTag::new_from_descs(v.as_slice())
                ));
            },
        }
        let mut v = Vec::new();
        v.resize(count, EfiMemoryDescriptor::default());
        v
    }

    pub fn new_module<'a>(&self, start: u32, end: u32, cmdline: Option<&'a str>) -> Module<'a> {
        match self {
            Self::Multiboot(_) => Module::Multiboot(MultibootModule::new(
                start.into(), end.into(), cmdline,
            )),
            Self::Multiboot2(_) => Module::Multiboot2(ModuleTag::new(
                start, end, cmdline.unwrap_or(""),
            )),
        }
    }

     pub fn set_boot_loader_name(&mut self, name: Option<&str>) {
        match self {
            Self::Multiboot(b) => b.with_wrap_mut(|w| w.set_boot_loader_name(name)),
            Self::Multiboot2(c) => if let Some(n) = name {
                c.update(|b| b.bootloader(BootLoaderNameTag::new(n)))
            },
        }
    }

    pub fn set_boot_services_not_exited(&mut self) {
        match self {
            // Multiboot1 doesn't know this.
            Self::Multiboot(_) => (),
            Self::Multiboot2(c) => c.update(|b| b.efi_bs(
                EFIBootServicesNotExitedTag::new()
            ))
        }
    }

    pub fn set_command_line(&mut self, cmdline: Option<&str>) {
        match self {
            Self::Multiboot(b) => b.with_wrap_mut(|w| w.set_command_line(cmdline)),
            Self::Multiboot2(cell) => if let Some(cmd) = cmdline {
                cell.update(|b| b.cmdline(CommandLineTag::new(cmd)));
            },
        }
    }

    pub fn set_efi_image_handle32(&mut self, pointer: u32) {
        match self {
            Self::Multiboot(_) => (), // Multiboot1 doesn't know about this
            Self::Multiboot2(c) => c.update(|b| b.efi32_ih(
                EFIImageHandle32Tag::new(pointer)
            )),
        }
    }

    pub fn set_efi_image_handle64(&mut self, pointer: u64) {
        match self {
            Self::Multiboot(_) => (), // Multiboot1 doesn't know about this
            Self::Multiboot2(c) => c.update(|b| b.efi64_ih(
                EFIImageHandle64Tag::new(pointer)
            )),
        }
    }

    pub fn set_memory_bounds(&mut self, bounds: Option<(u32, u32)>) {
        match self {
            Self::Multiboot(i) => i.with_wrap_mut(
                |w| w.set_memory_bounds(bounds)
            ),
            Self::Multiboot2(c) => if let Some((lower, upper)) = bounds {
                c.update(|b| b.meminfo(BasicMemoryInfoTag::new(lower, upper)));
            },
        }
    }

    pub fn set_framebuffer_table(&mut self, table: Option<FramebufferInfo>) {
        match self {
            Self::Multiboot(b) => b.with_wrap_mut(|w| w.set_framebuffer_table(
                table.map(|t| match t {
                    FramebufferInfo::Multiboot(i) => i,
                    FramebufferInfo::Multiboot2(_) => panic!("wrong Multiboot version"),
                })
            )),
            Self::Multiboot2(c) => if let Some(tab) = table {
                match tab {
                    FramebufferInfo::Multiboot(_) => panic!("wrong Multiboot version"),
                    FramebufferInfo::Multiboot2(t) => c.update(
                        |b| b.framebuffer(t)
                    ),
                }
            },
        }
    }

    pub fn set_image_load_addr(&mut self, addr: u32) {
        match self {
            Self::Multiboot(_) => (), // Multiboot1 doesn't know this
            Self::Multiboot2(c) => c.update(|b| b.image_load_addr(
                ImageLoadPhysAddrTag::new(addr)
            )),
        }
    }

    pub fn set_memory_regions(&mut self, regions: Option<&[MemoryEntry]>) {
        match self {
            Self::Multiboot(b) => b.set_memory_regions(regions),
            Self::Multiboot2(c) => if let Some(regs) = regions {
                    let v: Vec<_> = regs.iter().map(|me| match me {
                        MemoryEntry::Multiboot(_) => panic!("wrong Multiboot version"),
                        MemoryEntry::Multiboot2(ma) => *ma,
                    }).collect();
                    c.update(|b| b.mmap(MemoryMapTag::new(v.as_slice())));
            },
        }
    }

    pub fn set_modules(&mut self, modules: Option<Vec<Module>>) {
        match self {
            Self::Multiboot(b) => b.with_wrap_mut(|w| 
                match modules {
                    None => w.set_modules(None),
                    Some(mods) => {
                        let v: Vec<_> = mods.into_iter().map(|mo|match mo {
                            Module::Multiboot(m) => m,
                            Module::Multiboot2(_) => panic!("wrong Multiboot version"),
                        }).collect();
                        w.set_modules(Some(v.as_slice()));
                    }
                }
            ),
            Self::Multiboot2(c) => if let Some(mods) = modules {
                for mo in mods {
                    match mo {
                        Module::Multiboot(_) => panic!("wrong Multiboot version"),
                        Module::Multiboot2(m) => c.update(
                            |b| b.add_module(m)
                        ),
                    }
                }
            },
        }
    }

    pub fn set_rsdp_v1(
        &mut self, checksum: u8, oem_id: [u8; 6],
        revision: u8, rsdt_address: u32,
    ) {
        match self {
            Self::Multiboot(_) => (), // not supported on Multiboot1
            Self::Multiboot2(c) => c.update(|b| b.rsdpv1(RsdpV1Tag::new(
                checksum, oem_id, revision, rsdt_address,
            ))),
        }
    }

    pub fn set_rsdp_v2(
        &mut self, checksum: u8, oem_id: [u8; 6],
        revision: u8, rsdt_address: u32, length: u32, xsdt_address: u64,
        ext_checksum: u8,
    ) {
        match self {
            Self::Multiboot(_) => (), // not supported on Multiboot1
            Self::Multiboot2(c) => c.update(|b| b.rsdpv2(RsdpV2Tag::new(
                checksum, oem_id, revision, rsdt_address, length,
                xsdt_address, ext_checksum,
            ))),
        }
    }

    pub fn add_smbios_tag(&mut self, major: u8, minor: u8, tables: &[u8]) {
        match self {
            Self::Multiboot(_) => (), // not suppported on Multiboot1
            Self::Multiboot2(c) => c.update(|b| b.add_smbios(
                SmbiosTag::new(major, minor, tables)
            )),
        }
    }

    pub fn set_symbols(&mut self, symbols: Option<Symbols>) {
        match self {
            Self::Multiboot(b) => {
                b.with_wrap_mut(|w| w.set_symbols(symbols.map(|s| match s {
                    Symbols::Multiboot(t) => t,
                    Symbols::Multiboot2(_) => panic!("wrong Multiboot version"),
                })));
            },
            Self::Multiboot2(c) => if let Some(syms) = symbols {
                match syms {
                    Symbols::Multiboot(_) => panic!("wrong Multiboot version"),
                    Symbols::Multiboot2(sy) => if let Some(s) = sy {
                        c.update(|b| b.elf_sections(s));
                    }
                }
            },
        }
    }

    pub fn set_system_table_ia32(&mut self, systab: Option<u32>) {
        match self {
            Self::Multiboot(_) => (), // not suppported on Multiboot1
            Self::Multiboot2(c) => if let Some(st) = systab {
                c.update(|b| b.efi32(EFISdt32Tag::new(st)));
            },
        }
    }

    pub fn set_system_table_x64(&mut self, systab: Option<u64>) {
        match self {
            Self::Multiboot(_) => (), // not suppported on Multiboot1
            Self::Multiboot2(c) => if let Some(st) = systab {
                c.update(|b| b.efi64(EFISdt64Tag::new(st)));
            },
        }
    }
}

#[self_referencing]
pub struct MultibootInfoBuilder<A: Allocator + 'static> {
    info: MultibootInfo,
    allocator: MultibootAllocator<A>,
    memory_map_vec: Vec<MultibootMemoryEntry, A>,
    #[borrows(mut info, mut allocator)]
    #[not_covariant]
    wrap: Multiboot<'this, 'this>,
}

impl<A: Allocator> MultibootInfoBuilder<A> {
    fn allocate_memory_map_vec(&mut self, count: usize) {
        self.with_mut(|f| {
            f.memory_map_vec.resize(count, MultibootMemoryEntry::default());
            f.wrap.set_memory_regions(Some((f.memory_map_vec.as_slice().as_ptr() as PAddr, count)));
        });
    }

    fn set_memory_regions(&mut self, regions: Option<&[MemoryEntry]>) {
        self.with_mut(|s|
            match regions {
                None => s.wrap.set_memory_regions(None),
                Some(regs) => {
                    Self::copy_memory_regions(s.memory_map_vec, regs);
                    s.wrap.set_memory_regions(Some(
                        (s.memory_map_vec.as_slice().as_ptr() as PAddr, regs.len())
                    ));
                }
            }
        );
    }

    /// Write the entries into the vec.
    fn copy_memory_regions(memory_map_vec: &mut Vec<MultibootMemoryEntry, A>, regions: &[MemoryEntry]) {
        memory_map_vec.truncate(regions.len());
        regions.iter().zip(memory_map_vec.iter_mut()).for_each(
            |(source, destination)| match source {
                MemoryEntry::Multiboot(src) => *destination = *src,
                MemoryEntry::Multiboot2(_) => panic!("wrong Multiboot version"),
            }
        );
    }
}

/// Proxy Rust's allocator to the multiboot crate.
pub(super) struct MultibootAllocator<A: Allocator> {
    allocator: A,
    allocations: BTreeMap<u64, Layout>
}

impl<A: Allocator> MultibootAllocator<A> {
    /// Initialize the allocator.
    pub(super) const fn new(allocator: A) -> Self {
        Self { allocator, allocations: BTreeMap::new() }
    }
}

impl<A: Allocator> MemoryManagement for MultibootAllocator<A> {
    /// Get a slice to the memory referenced by the pointer.
    unsafe fn paddr_to_slice(
        &self, addr: u64, _length: usize
    ) -> Option<&'static [u8]> {
        // Using layout.size instead of length brings us safety, but may be too strict.
        self.allocations.get(&addr).map(|layout|
            core::slice::from_raw_parts(addr as *const u8, layout.size())
        )
    }

    /// Allocate n bytes of memory and return the address.
    unsafe fn allocate(
        &mut self, length: usize
    ) -> Option<(u64, &mut [u8])> {
        let layout = Layout::array::<u8>(length).expect("tried to allocate more than usize");
        let Ok(mut ptr) = self.allocator.allocate(layout) else { return None };
        if ptr.addr().get() >= u32::MAX as usize {
            return None
        }
        self.allocations.insert(ptr.addr().get() as u64, layout);
        Some((
            ptr.addr().get() as u64,
            ptr.as_mut(),
        ))
    }
    
    /// Free the previously allocated memory.
    unsafe fn deallocate(&mut self, addr: u64) {
        if addr == 0 {
            return;
        }
        match self.allocations.remove(&addr) {
            None => panic!(
                "couldn't free memory that has not been previously allocated: {addr}"
            ),
            Some(layout) => dealloc(addr as *mut u8, layout)
        }
    }
}

// TODO: Check whether the Clone breaks anything?
#[derive(Debug, Clone)]
pub enum MemoryEntry {
    Multiboot(MultibootMemoryEntry),
    Multiboot2(MemoryArea),
}

impl MemoryEntry {
    #[must_use]
    pub fn with(&self, base_addr: u64, length: u64, ty: MemoryType) -> Self {
        match self {
            Self::Multiboot(_) => Self::Multiboot(
                MultibootMemoryEntry::new(base_addr, length, MultibootMemoryType::from(ty))
            ),
            Self::Multiboot2(_) => Self::Multiboot2(
                MemoryArea::new(base_addr, length, MemoryAreaType::from(ty))
            ),
        }
    }

    #[must_use]
    pub fn base_address(&self) -> u64 {
        match self {
            Self::Multiboot(e) => e.base_address(),
            Self::Multiboot2(a) => a.start_address(),
        }
    }

    #[must_use]
    pub fn length(&self) -> u64 {
        match self {
            Self::Multiboot(e) => e.length(),
            Self::Multiboot2(a) => a.size(),
        }
    }

    #[must_use]
    pub fn memory_type(&self) -> MemoryType {
        match self {
            Self::Multiboot(e) => match e.memory_type() {
                MultibootMemoryType::Available => MemoryType::Available,
                MultibootMemoryType::Reserved => MemoryType::Reserved,
                MultibootMemoryType::ACPI => MemoryType::AcpiAvailable,
                MultibootMemoryType::NVS => MemoryType::ReservedHibernate,
                MultibootMemoryType::Defect => MemoryType::Defective,
            },
            Self::Multiboot2(a) => match a.typ().into() {
                MemoryAreaType::Available => MemoryType::Available,
                MemoryAreaType::Reserved => MemoryType::Reserved,
                MemoryAreaType::AcpiAvailable => MemoryType::AcpiAvailable,
                MemoryAreaType::ReservedHibernate => MemoryType::ReservedHibernate,
                MemoryAreaType::Defective => MemoryType::Defective,
                MemoryAreaType::Custom(_) => MemoryType::Reserved, // just to be sure
            },
        }
    }
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum MemoryType {
    Available,
    Reserved,
    AcpiAvailable,
    ReservedHibernate,
    Defective,
}

impl From<MemoryType> for MultibootMemoryType {
    fn from(info: MemoryType) -> Self {
        match info {
            MemoryType::Available => Self::Available,
            MemoryType::Reserved => Self::Reserved,
            MemoryType::AcpiAvailable => Self::ACPI,
            MemoryType::ReservedHibernate => Self::NVS,
            MemoryType::Defective => Self::Defect,
        }
    }
}

impl From<MemoryType> for MemoryAreaType {
    fn from(info: MemoryType) -> Self {
        match info {
            MemoryType::Available => Self::Available,
            MemoryType::Reserved => Self::Reserved,
            MemoryType::AcpiAvailable => Self::AcpiAvailable,
            MemoryType::ReservedHibernate => Self::ReservedHibernate,
            MemoryType::Defective => Self::Defective,
        }
    }
}

pub enum Module<'a> {
    Multiboot(MultibootModule<'a>),
    Multiboot2(Box<ModuleTag>),
}

pub enum Symbols {
    Multiboot(SymbolType),
    Multiboot2(Option<Box<ElfSectionsTag>>),
}

impl Symbols {
    pub(crate) fn new_multiboot(
        num: u32, size: u32, addr: usize, shndx: u32
    ) -> Self {
        Self::Multiboot(SymbolType::Elf(
            ElfSymbols::from_addr(
                num, size, addr.try_into().unwrap(), shndx,
            )
        ))
    }

    pub(crate) fn new_multiboot2(
        num: u32, size: u32, addr: usize, shndx: u32
    ) -> Self {
        let bytes = unsafe { slice::from_raw_parts(
            addr as *mut u8, (num * size).try_into().unwrap()
        ) };
        Self::Multiboot2(Some(ElfSectionsTag::new(
            num, size, shndx, bytes,
        )))
    }
}

pub enum ColorInfo {
    Multiboot(ColorInfoType),
    Multiboot2(FramebufferType<'static>),
}

impl ColorInfo {
    #[must_use]
    pub fn to_framebuffer_info(self,
        addr: u64,
        pitch: u32,
        width: u32,
        height: u32,
        bpp: u8,
    ) -> FramebufferInfo {
        match self {
            Self::Multiboot(c) => FramebufferInfo::Multiboot(
                FramebufferTable::new(addr, pitch, width, height, bpp, c)
            ),
            Self::Multiboot2(t) => FramebufferInfo::Multiboot2(
                FramebufferTag::new(addr, pitch, width, height, bpp, t)
            ),
        }
    }
}

#[derive(Debug)]
pub enum FramebufferInfo {
    Multiboot(FramebufferTable),
    Multiboot2(Box<FramebufferTag>),
}

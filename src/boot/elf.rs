//! Handling of ELF files

use core::convert::TryInto;
use alloc::collections::btree_map::BTreeMap;
use alloc::vec::Vec;

use log::{trace, debug, warn};

use goblin::elf;
use goblin::container;
use scroll::ctx::IntoCtx;

use multiboot::information::{ElfSymbols, SymbolType};

use super::super::mem::Allocation;

pub(super) struct OurElfLoader {
    // maps virtual to physical addresses
    allocations: BTreeMap<u64, Allocation>,
    virtual_entry_point: u64,
    physical_entry_point: Option<usize>,
}

impl OurElfLoader {
    /// Create a new instance.
    ///
    /// The parameter is the virtual address of the entry point.
    pub(super) fn new(entry_point: u64) -> Self {
        OurElfLoader {
            allocations: BTreeMap::new(),
            virtual_entry_point: entry_point,
            physical_entry_point: None,
        }
    }
    
    /// Load an ELF.
    pub(super) fn load_elf(&mut self, binary: &elf::Elf, data: &[u8]) -> Result<(), &'static str> {
        for program_header in &binary.program_headers {
            if program_header.p_type == elf::program_header::PT_LOAD {
                self.allocate(program_header)?;
                self.load(program_header.p_vaddr, &data[program_header.file_range()])?;
            }
        }
        Ok(())
    }
    
    /// Gets the entry point.
    ///
    /// We should have found it in `allocate`,
    /// else fall back to the virtual one and hope for the best.
    pub(super) fn entry_point(&self) -> usize {
        if let Some(a) = self.physical_entry_point {
            a
        } else {
            warn!("didn't find the entry point while loading sections, assuming virtual = physical");
            self.virtual_entry_point.try_into().unwrap()
        }
    }
    
    fn allocate(&mut self, header: &elf::program_header::ProgramHeader) -> Result<(), &'static str> {
            trace!("header: {:?}", header);
            debug!(
                "allocating {} {} bytes at {:#x} for {:#x}",
                header.p_memsz, header.p_flags, header.p_paddr, header.p_vaddr
            );
            let mut allocation = Allocation::new_at(
                header.p_paddr.try_into().unwrap(),
                header.p_memsz.try_into().unwrap(),
            ).map_err(|_e| "failed to allocate memory for the kernel")?;
            let mem_slice = allocation.as_mut_slice();
            mem_slice.fill(0);
            self.allocations.insert(header.p_vaddr, allocation);
            if header.p_vaddr <= self.virtual_entry_point
            && header.p_vaddr + header.p_memsz >= self.virtual_entry_point {
                self.physical_entry_point = Some(
                    (header.p_paddr + self.virtual_entry_point - header.p_vaddr)
                    .try_into().unwrap()
                );
                debug!(
                    "(this segment will contain the entry point {:#x} at {:#x})",
                    self.virtual_entry_point, self.physical_entry_point.unwrap(),
                );
            }
        Ok(())
    }
    
    fn load(&mut self, base: u64, region: &[u8]) -> Result<(), &'static str> {
        // check whether we actually allocated this
        match self.allocations.get_mut(&base) {
            None => panic!("we didn't allocate {:#x}, but tried to write to it o.O", base),
            Some(alloc) => {
                if alloc.len < region.len() {
                    panic!("{:#x} doesn't fit into the memory allocated for it", base);
                }
                let ptr = alloc.as_ptr();
                debug!(
                    "load {} bytes into {:#x} (at {:#x})", region.len(), base, ptr as usize
                );
                alloc.as_mut_slice()[0..region.len()].clone_from_slice(region);
                Ok(())
            },
        }
    }
}

impl From<OurElfLoader> for Vec<Allocation> {
    // Gets the allocated memory.
    fn from(loader: OurElfLoader) -> Vec<Allocation> {
        // using .values() would just borrow the values from the hash map
        loader.allocations.into_iter().map(|(_k, v)| v).collect()
    }
}

/// Bring the binary's symbols in a format for Multiboot.
///
/// Returns a tuple of informations struct and vector containing the symbols.
pub(super) fn symbols(binary: &mut elf::Elf, data: &[u8]) -> (SymbolType, Vec<u8>) {
    // Let's just hope they fit into u32s.
    let num: u32 = binary.header.e_shnum.into();
    let size: u32 = binary.header.e_shentsize.try_into().unwrap();
    
    // allocate memory to place the section headers and sections
    let mut memory = Vec::new();
    // reserve memory so that we don't have to re-allocate
    memory.reserve((
        u64::from(size * num)
        + binary.section_headers.iter().filter(|s| s.sh_addr == 0).map(|s| s.sh_size).sum::<u64>()
    ).try_into().unwrap());
    let ptr = memory.as_ptr();
    
    // copy the symbols
    // only copy sections that are not already loaded
    for mut section in binary.section_headers.iter_mut().filter(
        |s| s.sh_addr == 0 && s.file_range().is_some()
    ) {
        let index = memory.len();
        memory.extend_from_slice(&data[section.file_range().unwrap()]);
        section.sh_addr = (index + ptr as usize).try_into().unwrap();
        trace!("Loaded section {:?} to {:#x}", section, section.sh_addr);
    }
    
    // copy the section headers
    let shdr_begin = memory.len();
    // make sure that resizing won't reallocate
    assert!(memory.capacity() >= shdr_begin + (size * num) as usize);
    memory.resize(shdr_begin + (size * num) as usize, 0);
    // we can't copy from data as it still just contains null pointers
    let ctx = container::Ctx::new(
        if binary.is_64 { container::Container::Big } else { container::Container::Little },
        if binary.little_endian { container::Endian::Little } else { container::Endian::Big },
    );
    let mut begin_idx = shdr_begin;
    for section in &binary.section_headers {
        section.clone().into_ctx(&mut memory[begin_idx..begin_idx+size as usize], ctx);
        begin_idx += size as usize;
    }
    let shndx = binary.header.e_shstrndx.try_into().unwrap();
    (
        SymbolType::Elf(ElfSymbols::from_addr(
            num, size, (ptr as usize + shdr_begin) as multiboot::information::PAddr, shndx
        )),
        memory
    )
}

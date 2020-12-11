//! Handling of ELF files

use core::convert::TryInto;
use alloc::borrow::ToOwned;
use alloc::vec::Vec;

use log::{trace, debug, warn};

use hashbrown::HashMap;

use goblin::elf;

use multiboot::information::{ElfSymbols, SymbolType};

use super::super::mem::Allocation;

pub(super) struct OurElfLoader {
    // maps virtual to physical addresses
    allocations: HashMap<u64, Allocation>,
    virtual_entry_point: u64,
    physical_entry_point: Option<usize>,
}

impl OurElfLoader {
    /// Create a new instance.
    ///
    /// The parameter is the virtual address of the entry point.
    pub(super) fn new(entry_point: u64) -> Self {
        OurElfLoader {
            allocations: HashMap::new(),
            virtual_entry_point: entry_point,
            physical_entry_point: None,
        }
    }
    
    /// Load an ELF.
    pub(super) fn load_elf(&mut self, binary: &elf::Elf, data: &[u8]) -> Result<(), &'static str> {
        for program_header in &binary.program_headers {
            if program_header.p_type == elf::program_header::PT_LOAD {
                self.allocate(&program_header)?;
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
        match self.physical_entry_point {
            Some(a) => a,
            None => {
                warn!("didn't find the entry point while loading sections, assuming virtual = physical");
                self.virtual_entry_point.try_into().unwrap()
            },
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
            ).map_err(|e| "failed to allocate memory for the kernel")?;
            let mut mem_slice = allocation.as_mut_slice();
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

impl Into<Vec<Allocation>> for OurElfLoader {
    // Gets our allocated memory.
    fn into(mut self) -> Vec<Allocation> {
        self.allocations.into_iter().map(|(k, v)| v).collect()
    }
}

/// Bring the binary's symbols in a format for Multiboot.
///
/// Returns a tuple of informations struct and vector containing the symbols.
pub(super) fn symbols(binary: &elf::Elf, data: &[u8]) -> (SymbolType, Vec<u8>) {
    // Let's just hope they fit into u32s.
    let num: u32 = binary.header.e_shnum.into();
    let size: u32 = binary.header.e_shentsize.try_into().unwrap();
    // copy the section heades
    let section_vec: Vec<u8> = data.iter()
    .skip(binary.header.e_shoff.try_into().unwrap()).take((size * num).try_into().unwrap())
    .map(|b| b.to_owned()).collect();
    // TODO: actually copy the symbols
    let ptr = section_vec.as_ptr();
    let shndx = binary.header.e_shstrndx.try_into().unwrap();
    (
        SymbolType::Elf(ElfSymbols::from_addr(
            num, size, ptr as multiboot::information::PAddr, shndx
        )),
        section_vec
    )
}

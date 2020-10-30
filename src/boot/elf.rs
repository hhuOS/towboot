//! Handling of ELF files

use core::convert::TryInto;
use alloc::borrow::ToOwned;
use alloc::vec::Vec;

use log::{trace, debug, warn};

use hashbrown::HashMap;

use elfloader::{ElfBinary, ElfLoader, Flags, LoadableHeaders, P64, Rela, VAddr};

use multiboot::ElfSymbols;

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
}

impl Into<Vec<Allocation>> for OurElfLoader {
    // Gets our allocated memory.
    fn into(mut self) -> Vec<Allocation> {
        self.allocations.into_iter().map(|(k, v)| v).collect()
    }
}

impl ElfLoader for OurElfLoader {
    fn allocate(&mut self, load_headers: LoadableHeaders) -> Result<(), &'static str> {
        for header in load_headers {
            trace!("header: {:?}", header);
            debug!(
                "allocating {} {} bytes at {:#x} for {:#x}",
                header.mem_size(), header.flags(), header.physical_addr(), header.virtual_addr()
            );
            let mut allocation = Allocation::new_at(
                header.physical_addr().try_into().unwrap(),
                header.mem_size().try_into().unwrap(),
            ).map_err(|e| "failed to allocate memory for the kernel")?;
            let mut mem_slice = allocation.as_mut_slice();
            mem_slice.fill(0);
            self.allocations.insert(header.virtual_addr(), allocation);
            if header.virtual_addr() <= self.virtual_entry_point
            && header.virtual_addr() + header.mem_size() >= self.virtual_entry_point {
                self.physical_entry_point = Some(
                    (header.physical_addr() + self.virtual_entry_point - header.virtual_addr())
                    .try_into().unwrap()
                );
                debug!(
                    "(this segment will contain the entry point {:#x} at {:#x})",
                    self.virtual_entry_point, self.physical_entry_point.unwrap(),
                );
            }
        }
        Ok(())
    }

    fn relocate(&mut self, entry: &Rela<P64>) -> Result<(), &'static str> {
        unimplemented!("no support for ELF relocations");
    }

    fn load(&mut self, flags: Flags, base: VAddr, region: &[u8]) -> Result<(), &'static str> {
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

/// Bring the binary's symbols in a format for Multiboot.
pub(super) fn symbols(binary: &ElfBinary) -> ElfSymbols {
    // We need the section header part of the ELF header
    let header_part = binary.file.header.pt2;
    // Let's just hope they fit into u32s.
    let num: u32 = header_part.sh_count().into();
    let size: u32 = header_part.sh_entry_size().try_into().unwrap();
    // copy the symbols, FIXME: this leaks memory
    let section_vec: Vec<u8> = binary.file.input.iter()
    .skip(header_part.sh_offset().try_into().unwrap()).take((size * num).try_into().unwrap())
    .map(|b| b.to_owned()).collect();
    let ptr = section_vec.leak().as_ptr();
    let shndx = header_part.sh_str_index().try_into().unwrap();
    ElfSymbols::from_ptr(num, size, ptr, shndx)
}

//! Handling of ELF files

use core::convert::TryInto;
use alloc::borrow::ToOwned;
use alloc::vec::Vec;

use log::{trace, debug};

use elfloader::{ElfBinary, ElfLoader, Flags, LoadableHeaders, P64, Rela, VAddr};

use multiboot::ElfSymbols;

use super::super::mem::Allocation;

pub(super) struct OurElfLoader {
    // be careful, they have to be freed!
    pub(super) allocations: Vec<Allocation>,
}

impl OurElfLoader {
    pub(super) fn new() -> Self {
        OurElfLoader { allocations: Vec::new() }
    }
}

impl ElfLoader for OurElfLoader {
    fn allocate(&mut self, load_headers: LoadableHeaders) -> Result<(), &'static str> {
        for header in load_headers {
            if header.virtual_addr() != header.physical_addr() {
                todo!("support loading ELFs where virtual_addr != physical_addr")
            }
            trace!("header: {:?}", header);
            debug!(
                "allocating {} {} bytes at {:#x}",
                header.mem_size(), header.flags(), header.physical_addr()
            );
            let mut allocation = Allocation::new_at(
                header.physical_addr().try_into().unwrap(),
                header.mem_size().try_into().unwrap(),
            ).map_err(|e| "failed to allocate memory for the kernel")?;
            let mut mem_slice = allocation.as_mut_slice();
            mem_slice.fill(0);
            self.allocations.push(allocation);
        }
        Ok(())
    }

    fn relocate(&mut self, entry: &Rela<P64>) -> Result<(), &'static str> {
        unimplemented!("no support for ELF relocations");
    }

    fn load(&mut self, flags: Flags, base: VAddr, region: &[u8]) -> Result<(), &'static str> {
        // check whether we actually allocated this
        if !self.allocations.iter().any(|a| a.contains(base, region.len())) {
            panic!("we didn't allocate {:#x}, but tried to write to it o.O", base);
        }
        debug!("load {} bytes into {:#x}", region.len(), base);
        let mut mem_slice = unsafe {
            core::slice::from_raw_parts_mut(base as *mut u8, region.len())
        };
        mem_slice.clone_from_slice(region);
        Ok(())
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

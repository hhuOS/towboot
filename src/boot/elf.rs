//! Handling of ELF files

use core::convert::TryInto;
use alloc::vec::Vec;

use uefi::prelude::*;

use log::{trace, debug};

use elfloader::{ElfLoader, Flags, LoadableHeaders, P64, Rela, VAddr};

use super::mem::Allocation;

pub(super) struct OurElfLoader<'a> {
    // be careful, they have to be freed!
    pub(super) allocations: Vec<Allocation>,
    systab: &'a SystemTable<Boot>
}

impl<'a> OurElfLoader<'a> {
    pub(super) fn new(systab: &'a SystemTable<Boot>) -> Self {
        OurElfLoader {
            allocations: Vec::new(),
            systab
        }
    }
}

impl<'a> ElfLoader for OurElfLoader<'a> {
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
                &self.systab
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

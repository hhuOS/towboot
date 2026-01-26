use core::alloc::Allocator;
use core::pin::Pin;
use alloc::boxed::Box;
use ouroboros::self_referencing;

use multiboot::header::{
    Header as MultibootHeader,
    MultibootAddresses,
    MultibootVideoMode,
    VideoModeType,
};
use multiboot2_header::{
    AddressHeaderTag,
    ConsoleHeaderTag,
    FramebufferHeaderTag,
    Multiboot2BasicHeader,
    Multiboot2Header,
};

use super::information::{InfoBuilder, Symbols};

#[derive(Debug)]
pub enum Header {
    Multiboot(MultibootHeader),
    Multiboot2(Multiboot2HeaderWrap),
}

impl Header {
    pub fn from_slice(buffer: &[u8]) -> Option<Self> {
        match Multiboot2HeaderWrap::from_slice(buffer) {
            Some(w) => Some(Self::Multiboot2(w)),
            None => MultibootHeader::from_slice(buffer).map(Self::Multiboot),
        }
    }

    #[must_use]
    pub fn header_start(&self) -> u32 {
        match self {
            Self::Multiboot(h) => h.header_start,
            Self::Multiboot2(h) => *h.borrow_header_start(),
        }
    }

    pub fn get_preferred_video_mode(&self) -> Option<VideoMode<'_>> {
        match self {
            Self::Multiboot(h) => h.get_preferred_video_mode().map(VideoMode::Multiboot),
            Self::Multiboot2(h) => {
                if let Some(fb) = h.borrow_header().framebuffer_tag() {
                    Some(VideoMode::Multiboot2(Multiboot2VideoMode::LinearGraphics(fb)))
                } else {
                    h.borrow_header().console_flags_tag().map(
                        |ct| VideoMode::Multiboot2(Multiboot2VideoMode::TextMode(ct))
                    )
                }
            }
        }
    }

    pub fn get_load_addresses(&self) -> Option<Addresses> {
        match self {
            Self::Multiboot(h) => h.get_addresses().map(Addresses::Multiboot),
            Self::Multiboot2(h) => {
                h.borrow_header()
                    .address_tag()
                    .map(|a| Addresses::Multiboot2(*a))
            }
        }
    }

    #[must_use]
    pub fn get_efi32_entry_address(&self) -> Option<u32> {
        match self {
            Self::Multiboot(_) => None, // Multiboot1 doesn't support this
            Self::Multiboot2(w) => w.borrow_header().entry_address_efi32_tag()
                .map(|t| t.entry_addr()),
        }
    }

    #[must_use]
    pub fn get_efi64_entry_address(&self) -> Option<u32> {
        match self {
            Self::Multiboot(_) => None, // Multiboot1 doesn't support this
            Self::Multiboot2(w) => w.borrow_header().entry_address_efi64_tag()
                .map(|t| t.entry_addr()),
        }
    }

    #[must_use]
    pub fn get_entry_address(&self) -> Option<u32> {
        match self {
            Self::Multiboot(h) => h.get_addresses().map(
                |a| a.entry_address
            ),
            Self::Multiboot2(h) => {
                h.borrow_header().entry_address_tag().map(
                    |t| t.entry_addr()
                )
            }
        }
    }

    #[must_use]
    pub fn info_builder<A: Allocator + Clone>(&self, allocator: A) -> InfoBuilder<A> {
        match self {
            Self::Multiboot(_) => InfoBuilder::new_multiboot(allocator),
            Self::Multiboot2(_) => InfoBuilder::new_multiboot2(),
        }
    }

    #[must_use]
    pub fn new_elf_symbols(
        &self, num: u32, size: u32, addr: usize, shndx: u32
    ) -> Symbols {
        match self {
            Self::Multiboot(_) => Symbols::new_multiboot(
                num, size, addr, shndx
            ),
            Self::Multiboot2(_) => Symbols::new_multiboot2(
                num, size, addr, shndx
            ),
        }
    }

    #[must_use]
    pub fn should_exit_boot_services(&self) -> bool {
        match self {
            Self::Multiboot(_) => true, // Multiboot1 doesn't know about this
            Self::Multiboot2(w) => w.borrow_header().efi_boot_services_tag()
                .is_none(),
        }
    }
}

#[self_referencing]
#[derive(Debug)]
pub struct Multiboot2HeaderWrap {
    header_pin: Pin<Box<[u8]>>,
    header_start: u32,
    #[borrows(header_pin)]
    #[covariant]
    header: Multiboot2Header<'this>,
}

impl Multiboot2HeaderWrap {
    fn from_slice(buffer: &[u8]) -> Option<Self> {
        // first, find the header
        let (header_buf, header_start) = Multiboot2Header::find_header(buffer).ok()??;
        // then, copy it
        let header_pin = Box::into_pin(header_buf.to_vec().into_boxed_slice());
        Some(Multiboot2HeaderWrapBuilder {
            header_pin,
            header_start,
            header_builder: |header_pin: &Pin<Box<[u8]>>| unsafe {
                // yes, that's bad, but making it better would mean modifying
                // the multiboot2 crate
                Multiboot2Header::load(
                    header_pin.as_ref().as_ptr().cast::<Multiboot2BasicHeader>()
                ).unwrap() // `find_header` should have failed already.
            }
        }.build())
    }
}


pub enum Addresses {
    Multiboot(MultibootAddresses),
    Multiboot2(AddressHeaderTag),
}

impl Addresses {
    #[must_use]
    pub fn compute_load_offset(&self, header_start: u32) -> u32 {
        match self {
            Self::Multiboot(a) => a.compute_load_offset(header_start),
            Self::Multiboot2(a) => header_start - (
                a.header_addr() - a.load_addr()
            ),
        }
    }

    #[must_use]
    pub const fn compute_kernel_length(&self, whole_length: u32) -> u32 {
        if self.bss_end_addr() == 0 {
            if self.load_end_addr() == 0 {
                self.header_addr() + whole_length - self.load_addr()
            } else {
                self.load_end_addr() - self.load_addr()
            }
        } else {
            self.bss_end_addr() - self.load_addr()
        }
    }

    const fn header_addr(&self) -> u32 {
        match self {
            Self::Multiboot(a) => a.header_address,
            Self::Multiboot2(a) => a.header_addr(),
        }
    }

    const fn bss_end_addr(&self) -> u32 {
        match self {
            Self::Multiboot(a) => a.bss_end_address,
            Self::Multiboot2(a) => a.bss_end_addr(),
        }
    }

    #[must_use]
    pub const fn load_addr(&self) -> u32 {
        match self {
            Self::Multiboot(a) => a.load_address,
            Self::Multiboot2(a) => a.load_addr(),
        }
    }

    #[must_use]
    pub const fn load_end_addr(&self) -> u32 {
        match self {
            Self::Multiboot(a) => a.load_end_address,
            Self::Multiboot2(a) => a.load_end_addr(),
        }
    }
}

pub enum VideoMode<'a> {
    Multiboot(MultibootVideoMode),
    Multiboot2(Multiboot2VideoMode<'a>),
}

impl VideoMode<'_> {
    #[must_use]
    pub fn is_graphics(&self) -> bool {
        match self {
            Self::Multiboot(vm) => matches!(
                vm.mode_type(), Some(VideoModeType::LinearGraphics),
            ),
            Self::Multiboot2(Multiboot2VideoMode::LinearGraphics(_)) => true,
            _ => false,
        }
    }

    #[must_use]
    pub fn depth(&self) -> Option<u32> {
        match self {
            Self::Multiboot(vm) => vm.depth(),
            Self::Multiboot2(Multiboot2VideoMode::LinearGraphics(&ft)) => {
                Some(ft.depth())
            },
            _ => None,
        }
    }

    /// Return the width of the framebuffer.
    /// Text consoles in multiboot2 have no size.
    #[must_use]
    pub const fn width(&self) -> Option<u32> {
        match self {
            Self::Multiboot(vm) => Some(vm.width),
            Self::Multiboot2(Multiboot2VideoMode::LinearGraphics(&ft)) => {
                Some(ft.width())
            },
            Self::Multiboot2(Multiboot2VideoMode::TextMode(_)) => None,
        }
    }

    /// Return the height of the framebuffer.
    /// Text consoles in multiboot2 have no size.
    #[must_use]
    pub const fn height(&self) -> Option<u32> {
        match self {
            Self::Multiboot(vm) => Some(vm.height),
            Self::Multiboot2(Multiboot2VideoMode::LinearGraphics(&ft)) => {
                Some(ft.height())
            },
            Self::Multiboot2(Multiboot2VideoMode::TextMode(_)) => None,
        }
    }
}


pub enum Multiboot2VideoMode<'a> {
    LinearGraphics(&'a FramebufferHeaderTag),
    TextMode(&'a ConsoleHeaderTag),
}

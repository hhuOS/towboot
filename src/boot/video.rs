//! Management of the video mode.

use alloc::collections::btree_set::BTreeSet;
use alloc::vec::Vec;

use uefi::prelude::*;
use uefi::proto::console::gop::{GraphicsOutput, Mode, PixelBitmask, PixelFormat};
use uefi::table::boot::ScopedProtocol;

use log::{debug, warn, info, error};

use multiboot12::header::Header;
use multiboot12::information::{InfoBuilder, ColorInfo};

use super::super::config::Quirk;

/// Try to get the video in a mode the kernel wants.
///
/// If there are multiple GPUs available, simply choose the first one.
/// If there is no available mode that matches, just use the one we're already in.
pub(super) fn setup_video<'a>(
    header: &Header, systab: &'a SystemTable<Boot>, quirks: &BTreeSet<Quirk>
) -> Option<ScopedProtocol<'a, GraphicsOutput<'a>>> {
    info!("setting up the video...");
    let wanted_resolution = match (
        header.get_preferred_video_mode(), quirks.contains(&Quirk::KeepResolution)
    ) {
        (Some(mode), false) => {
            if mode.is_graphics() {
                // lets just hope that the firmware supports 24-bit RGB
                // the other modes are way too obscure
                // 0 means "no preference"
                if mode.depth().unwrap() != 24 || mode.depth().unwrap() == 0 {
                    warn!(
                        "color depth will be 24-bit, but the kernel wants {}",
                        mode.depth().unwrap()
                    );
                }
                Some((mode.width().unwrap(), mode.height().unwrap()))
            } else {
                // We could set the console to this resolution,
                // but if the kernel doesn't have any EFI support, it won't be able to use it.
                // So, just chose a video mode and hope that the kernel supports video.
                // TODO: Perhaps support EFI text mode later on.
                warn!("text mode is not implemented");
                None
            }
        },
        _ => None,
    };
    // just get the first one
    let handles = systab
        .boot_services()
        .find_handles::<GraphicsOutput>()
        .expect("failed to list available graphics outputs");
    let handle = handles.get(0).or_else(|| {
        warn!("Failed to find a graphics output. Do you have a graphics card (and a driver)?");
        None
    })?;
    let mut output: ScopedProtocol<GraphicsOutput> = systab.boot_services().open_protocol_exclusive(*handle).ok()?;
    let modes: Vec<Mode> = output.modes().collect();
    debug!(
        "available video modes: {:?}",
        modes.iter().map(Mode::info).map(|i| (i.resolution(), i.pixel_format()))
        .collect::<Vec<((usize, usize), PixelFormat)>>()
    );
    // try to see, if we find a matching mode
    if let Some(mode) = match wanted_resolution {
        Some((w, h)) => {
            modes.iter().find(|m|
                m.info().resolution() == (w as usize, h as usize)
            ).or_else(|| {
                warn!("failed to find a matching video mode (kernel wants {w}x{h})");
                None
            })
        },
        None => None,
    // in that case: set it
    } {
        debug!("chose {:?} as the video mode", mode.info().resolution());
        output.set_mode(mode).map_err(|e| {
            error!("failed to set video mode: {e:?}");
            Status::DEVICE_ERROR
        }).ok()?;
        info!("set {:?} as the video mode", mode.info().resolution());
    }
    Some(output)
}

/// Pass the framebuffer information to the kernel.
pub(super) fn prepare_information(
    multiboot: &mut InfoBuilder, mut graphics_output: ScopedProtocol<GraphicsOutput>,
) {
    let address = graphics_output.frame_buffer().as_mut_ptr();
    let mode = graphics_output.current_mode_info();
    debug!("gop mode: {mode:?}");
    let (width, height) = mode.resolution();
    let mut bpp = 32;
    let color_info = match mode.pixel_format() {
        PixelFormat::Rgb => multiboot.new_color_info_rgb(
            0,
            8,
            8,
            8,
            6,
            8,
        ),
        PixelFormat::Bgr => multiboot.new_color_info_rgb(
            16,
            8,
            8,
            8,
            0,
            8,
        ),
        PixelFormat::Bitmask => {
            let bitmask = mode.pixel_bitmask().unwrap();
            bpp = bitmask_to_bpp(bitmask);
            bitmask_to_color_info(multiboot, bitmask)
        },
        PixelFormat::BltOnly => panic!("GPU doesn't support pixel access"),
    };
    let pitch = mode.stride() * (bpp / 8) as usize;
    let framebuffer_table = color_info.to_framebuffer_info(
        address as u64,
        pitch.try_into().unwrap(),
        width.try_into().unwrap(),
        height.try_into().unwrap(),
        bpp,
    );
    debug!("passing {framebuffer_table:?}");
    multiboot.set_framebuffer_table(Some(framebuffer_table));
}

/// Converts UEFI's `PixelBitmask` to Multiboot's `ColorInfoRGB`.
fn bitmask_to_color_info(
    info_builder: &InfoBuilder, pixel_bitmask: PixelBitmask
) -> ColorInfo {
    let (red_field_position, red_mask_size) = parse_color_bitmap(pixel_bitmask.red);
    let (green_field_position, green_mask_size) = parse_color_bitmap(pixel_bitmask.green);
    let (blue_field_position, blue_mask_size) = parse_color_bitmap(pixel_bitmask.blue);
    info_builder.new_color_info_rgb(
        red_field_position, red_mask_size,
        green_field_position, green_mask_size,
        blue_field_position, blue_mask_size,
    )
}

macro_rules! check_bit {
    ($var:expr, $bit:expr) => {
        ($var & (1 << $bit) == (1 << $bit))
    };
}

/// Converts UEFI's `PixelBitmask` to Multiboot's `bpp` (bits per pixel).
fn bitmask_to_bpp(pixel_bitmask: PixelBitmask) -> u8 {
    let combined_bitmask = pixel_bitmask.red | pixel_bitmask.green | pixel_bitmask.blue;
    assert_eq!(pixel_bitmask.red & pixel_bitmask.green, 0);
    assert_eq!(pixel_bitmask.red & pixel_bitmask.blue, 0);
    assert_eq!(pixel_bitmask.green & pixel_bitmask.blue, 0);
    let mut bpp = 0;
    for i in 0..31 {
        if check_bit!(combined_bitmask, i) {
            bpp += 1;
        }
    }
    bpp
}

/// Converts a bitmask into a tuple of `field_position`, `mask_size`.
fn parse_color_bitmap(bitmask: u32) -> (u8, u8) {
    // find the first set bit
    let mut field_position = 0;
    for i in 0..31 {
        if check_bit!(bitmask, i) {
            field_position = i;
            break;
        }
    }
    // count how many bits are set
    let mut mask_size = 0;
    for i in field_position..31 {
        if !check_bit!(bitmask, i) {
            break;
        }
        mask_size += 1;
    }
    // check whether there are remaining bits set
    for i in field_position+mask_size..31 {
        if check_bit!(bitmask, i) {
            panic!("color bitmask is not continuous");
        }
    }
    (field_position, mask_size)
}

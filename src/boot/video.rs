//! Management of the video mode.

use alloc::vec::Vec;

use uefi::prelude::*;
use uefi::proto::console::gop::{GraphicsOutput, Mode, PixelFormat};

use log::{debug, warn, info, error};

use multiboot::{Header, VideoModeType};

/// Try to get the video in a mode the kernel wants.
///
/// If there are multiple GPUs available, simply choose the first one.
/// If there is no available mode that matches, choose one.
pub(super) fn setup_video<'a>(
    header: &Header, systab: &'a SystemTable<Boot>
) -> Result<&'a GraphicsOutput<'a>, Status> {
    info!("setting up the video");
    let wanted_resolution = match header.get_preferred_video_mode() {
        Some(mode) => match mode.mode_type() {
            Some(VideoModeType::LinearGraphics) => {
                // lets just hope that the firmware supports 24-bit RGB
                // the other modes are way too obscure
                // 0 means "no preference"
                if mode.depth().unwrap() != 24 || mode.depth().unwrap() == 0 {
                    warn!(
                        "color depth will be 24-bit, but the kernel wants {}",
                        mode.depth().unwrap()
                    );
                }
                Some((mode.width, mode.height))
            },
            Some(VideoModeType::TextMode) => {
                // We could set the console to this resolution,
                // but if the kernel doesn't have any EFI support, it won't be able to use it.
                // So, just chose a video mode and hope that the kernel supports video.
                // TODO: Perhaps support EFI text mode later on.
                warn!("text mode is not implemented");
                None
            },
            None => {
                warn!("kernel wants unknown video mode");
                None
            },
        },
        None => None,
    };
    // just get the first one
    let output = systab.boot_services().locate_protocol::<GraphicsOutput>().map_err(|e| {
        error!(
            "Failed to find a graphics output. Do you have a graphics card (and a driver)?: {:?}",
            e
        );
        Status::DEVICE_ERROR
    })?.log();
    let output = unsafe { &mut *output.get() };
    let modes: Vec<Mode> = output.modes().map(|c| c.log()).collect();
    debug!(
        "available video modes: {:?}",
        modes.iter().map(|m| m.info()).map(|i| (i.resolution(), i.pixel_format()))
        .collect::<Vec<((usize, usize), PixelFormat)>>()
    );
    let mode = match match wanted_resolution {
        Some((w, h)) => {
            modes.iter().find(|m|
                m.info().resolution() == (w as usize, h as usize)
            ).or_else(|| {
                warn!("failed to find a matching video mode (kernel wants {}x{})", w, h);
                None
            })
        },
        None => None,
    }{
        Some(mode) => Ok(mode),
        None => {
            // just choose the last one, it might have the biggest resolution
            modes.iter().last().ok_or_else(|| {
                error!("no video modes available");
                Status::DEVICE_ERROR
            })
        }
    }?;
    debug!("chose {:?} as the video mode", mode.info().resolution());
    output.set_mode(&mode).map_err(|e| {
        error!("failed to set video mode: {:?}", e);
        Status::DEVICE_ERROR
    })?.log();
    info!("set {:?} as the video mode", mode.info().resolution());
    Ok(output)
}

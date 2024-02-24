//! This crate offers functionality to use towboot for your own operating system.
use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use tempfile::NamedTempFile;

use towboot_config::Config;

mod bochs;
pub mod config;
mod image;
pub use bochs::bochsrc;
pub use image::Image;

/// Write the given configuration file to image.
/// This also copies all files that are referenced in it.
pub fn add_config_to_image(image: &mut Image, config: &mut Config) -> Result<()> {
    let mut config_path = PathBuf::from(config.src.clone());
    config_path.pop();
    // go through all needed files; including them (but without the original path)
    for src_file in config.needed_files() {
        let src_path = config_path.join(PathBuf::from(&src_file));
        let dst_file = src_path.file_name().unwrap();
        let dst_path = PathBuf::from(&dst_file);
        src_file.clear();
        src_file.push_str(dst_file.to_str().unwrap());
        image.add_file(&src_path, &dst_path)?;
    }

    // write the configuration itself to the image
    let mut config_file = NamedTempFile::new()?;
    config_file.as_file_mut().write_all(
        toml::to_string(&config)?.as_bytes()
    )?;
    image.add_file(&config_file.into_temp_path(), &PathBuf::from("towboot.toml"))?;
    Ok(())
}

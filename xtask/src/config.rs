// TODO: get this directly from towboot?

use std::{collections::BTreeMap, path::PathBuf, io::{Error, Read, ErrorKind}, fs::File};

use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct Config {
    pub entries: BTreeMap<String, Entry>,
}

#[derive(Deserialize, Debug)]
pub struct Entry {
    pub image: PathBuf,
    #[serde(default)]
    pub modules: Vec<Module>,
}

#[derive(Deserialize, Debug)]
pub struct Module {
    pub image: PathBuf,
}

pub(super) fn get_files_for_config(config: &PathBuf) -> Result<Vec<PathBuf>, Error> {
    let mut contents = String::new();
    File::open(config)?.read_to_string(&mut contents)?;
    let config: Config = toml::from_str(&contents)
        .map_err(|err| Error::new(ErrorKind::Other, err))?;
    let mut files = Vec::new();
    for (_name, entry) in config.entries.iter() {
        files.push(entry.image.clone());
        for module in &entry.modules {
            files.push(module.image.clone());
        }
    }
    Ok(files)
}

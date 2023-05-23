use std::{collections::HashMap, fs::File};

use serde::Deserialize;
use thiserror::Error;
use turbopath::AbsoluteSystemPathBuf;

struct Package {
    name: String,
    // package: NpmPackage,
    path: AbsoluteSystemPathBuf,
}

#[derive(Deserialize)]
struct PackageJson {
    name: String,
}

struct Context {
    packages: HashMap<String, Package>,
}

#[derive(Error, Debug)]
enum DiscoveryError {
    #[error("error when setting up walk: {0}")]
    Globwalk(#[from] globwalk::WalkError),
    #[error("error when walking fs: {0}")]
    WalkDir(#[from] globwalk::WalkDirError),
    #[error("error when reading json: {0}")]
    Io(#[from] std::io::Error),
    #[error("error when parsing json: {0}")]
    Parse(#[from] serde_json::Error),
}

impl Context {
    pub fn discover(base_path: &AbsoluteSystemPathBuf) -> Result<Self, DiscoveryError> {
        let include = ["**/package.json".to_string()];
        let walker = globwalk::globwalk(base_path, &include, &[], globwalk::WalkType::Files)?;
        let packages = walker
            .map(|path| {
                path.map_err(DiscoveryError::WalkDir).and_then(|path| {
                    let reader = File::open(path.as_path())?;
                    let json: PackageJson = serde_json::from_reader(reader)?;
                    Ok((
                        json.name.clone(),
                        Package {
                            name: json.name,
                            path,
                        },
                    ))
                })
            })
            .collect::<Result<HashMap<_, _>, _>>()?;

        Ok(Self { packages })
    }
}

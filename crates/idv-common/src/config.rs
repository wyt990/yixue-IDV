use std::fs;
use std::path::Path;

use serde::de::DeserializeOwned;

use crate::error::{Error, Result};

/// Load a TOML configuration file into `T`.
pub fn load_toml_file<T: DeserializeOwned>(path: impl AsRef<Path>) -> Result<T> {
    let path = path.as_ref();
    let raw = fs::read_to_string(path)
        .map_err(|err| Error::Config(format!("failed to read {}: {err}", path.display())))?;
    toml::from_str(&raw)
        .map_err(|err| Error::Config(format!("failed to parse {}: {err}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::io::Write;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Sample {
        name: String,
    }

    #[test]
    fn loads_valid_toml() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("idv-common-config-{}.toml", uuid::Uuid::new_v4()));
        {
            let mut file = fs::File::create(&path).expect("create temp toml");
            writeln!(file, "name = \"yixue\"").expect("write temp toml");
        }
        let sample: Sample = load_toml_file(&path).expect("parse sample");
        assert_eq!(sample.name, "yixue");
        let _ = fs::remove_file(path);
    }
}

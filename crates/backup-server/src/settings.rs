use std::path::PathBuf;

use backup_core::{Error, Result};
use backup_pg::ZstdLevel;

const DEFAULT_DATA_FOLDER: &str = "data";

#[derive(Debug, Clone)]
pub struct Settings {
    pub drive_creds: String,
    pub comp_level: ZstdLevel,
    pub drive_folder_id: String,
    /// Temporary dump files live here.
    pub data_folder: PathBuf,
}

impl Settings {
    pub fn from_env() -> Result<Self> {
        Self::from_source(|key| std::env::var(key).ok())
    }

    pub fn from_source(get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let drive_creds =
            get("DRIVE_CREDS").ok_or_else(|| Error::Config("DRIVE_CREDS is required".into()))?;
        let comp_level = match get("COMPRESSION_LEVEL") {
            None => ZstdLevel::default(),
            Some(v) => {
                let level = v
                    .parse::<i64>()
                    .map_err(|e| Error::Config(format!("COMPRESSION_LEVEL must be int: {e}")))?;
                ZstdLevel::new(level)?
            }
        };

        Ok(Self {
            drive_creds,
            comp_level,
            drive_folder_id: get("DRIVE_FOLDER_ID")
                .ok_or_else(|| Error::Config("DRIVE_FOLDER_ID is required".into()))?,
            data_folder: get("DATA_FOLDER")
                .unwrap_or_else(|| DEFAULT_DATA_FOLDER.to_string())
                .into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;

    use backup_pg::ZstdLevel;

    use super::Settings;

    fn source(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn test_parse_ok_with_defaults() {
        let s = Settings::from_source(source(&[("DRIVE_CREDS", "{}"), ("DRIVE_FOLDER_ID", "f")]))
            .unwrap();

        assert_eq!(s.comp_level, ZstdLevel::default());
        assert_eq!(s.drive_folder_id, "f");
        assert_eq!(s.data_folder, Path::new("data"));
    }

    #[test]
    fn test_parse_fails_without_drive_creds() {
        assert!(Settings::from_source(source(&[])).is_err());
    }

    #[test]
    fn test_parse_fails_without_drive_folder_id() {
        assert!(Settings::from_source(source(&[("DRIVE_CREDS", "{}")])).is_err());
    }

    #[test]
    fn test_parse_fails_on_not_int_compression_level() {
        let src = source(&[
            ("DRIVE_CREDS", "{}"),
            ("DRIVE_FOLDER_ID", "f"),
            ("COMPRESSION_LEVEL", "3.14"),
        ]);

        assert!(Settings::from_source(src).is_err());
    }

    #[test]
    fn test_parse_fails_on_out_of_range_compression_level() {
        let src = source(&[
            ("DRIVE_CREDS", "{}"),
            ("DRIVE_FOLDER_ID", "f"),
            ("COMPRESSION_LEVEL", "42"),
        ]);

        assert!(Settings::from_source(src).is_err());
    }
}

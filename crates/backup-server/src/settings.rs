use backup_core::Errors;

const DEFAULT_COMPRESSION_LEVEL: i32 = 3;
const COMPRESSION_LEVELS: std::ops::RangeInclusive<i32> = 1..=22;

#[derive(Debug, Clone)]
pub struct Settings {
    pub drive_creds: String,
    pub comp_level: i32,
    pub drive_folder_id: Option<String>,
}

impl Settings {
    pub fn from_env() -> Result<Self, Errors> {
        Self::from_source(|key| std::env::var(key).ok())
    }

    pub fn from_source(get: impl Fn(&str) -> Option<String>) -> Result<Self, Errors> {
        let drive_creds =
            get("DRIVE_CREDS").ok_or_else(|| Errors::EnvError("DRIVE_CREDS is required".into()))?;
        let drive_folder_id = get("DRIVE_FOLDER_ID");
        let comp_level = match get("COMPRESSION_LEVEL") {
            None => DEFAULT_COMPRESSION_LEVEL,
            Some(v) => v
                .parse::<i32>()
                .map_err(|e| Errors::EnvError(format!("COMPRESSION_LEVEL must be int: {e}")))?,
        };
        if !COMPRESSION_LEVELS.contains(&comp_level) {
            return Err(Errors::EnvError(format!(
                "COMPRESSION_LEVEL must be in {COMPRESSION_LEVELS:?}, {comp_level} found"
            )));
        }

        Ok(Self {
            drive_creds,
            comp_level,
            drive_folder_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

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
        let s = Settings::from_source(source(&[("DRIVE_CREDS", "{}")])).unwrap();

        assert_eq!(s.comp_level, 3);
        assert_eq!(s.drive_folder_id, None);
    }

    #[test]
    fn test_parse_fails_without_drive_creds() {
        assert!(Settings::from_source(source(&[])).is_err());
    }

    #[test]
    fn test_parse_fails_on_not_int_compression_level() {
        let src = source(&[("DRIVE_CREDS", "{}"), ("COMPRESSION_LEVEL", "3.14")]);

        assert!(Settings::from_source(src).is_err());
    }

    #[test]
    fn test_parse_fails_on_out_of_range_compression_level() {
        let src = source(&[("DRIVE_CREDS", "{}"), ("COMPRESSION_LEVEL", "42")]);

        assert!(Settings::from_source(src).is_err());
    }
}

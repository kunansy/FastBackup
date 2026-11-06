use std::net::SocketAddr;
use std::path::PathBuf;

use backup_core::{DbTarget, Error, Result, Secret};
use backup_pg::ZstdLevel;

const DEFAULT_DATA_FOLDER: &str = "data";
const DEFAULT_DB_PORT: u16 = 5432;
const DEFAULT_LISTEN_ADDR: &str = "0.0.0.0:50051";

#[derive(Debug, Clone)]
pub struct Settings {
    pub listen_addr: SocketAddr,
    pub db: DbTarget,
    pub comp_level: ZstdLevel,
    pub drive_creds: Secret,
    pub drive_folder_id: String,
    /// Temporary dump files live here.
    pub data_folder: PathBuf,
    /// Keep this many regular pg_dump backups; `None` keeps all.
    pub retention: Option<usize>,
}

fn parse<T: std::str::FromStr>(key: &str, value: &str) -> Result<T>
where
    T::Err: std::fmt::Display,
{
    value
        .parse()
        .map_err(|e| Error::Config(format!("{key}: invalid value '{value}': {e}")))
}

impl Settings {
    pub fn from_env() -> Result<Self> {
        Self::from_source(|key| std::env::var(key).ok())
    }

    pub fn from_source(get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let required =
            |key: &str| get(key).ok_or_else(|| Error::Config(format!("{key} is required")));

        let db = DbTarget {
            host: required("DB_HOST")?,
            port: match get("DB_PORT") {
                Some(v) => parse("DB_PORT", &v)?,
                None => DEFAULT_DB_PORT,
            },
            user: required("DB_USERNAME")?,
            password: Secret::new(required("DB_PASSWORD")?),
            name: required("DB_NAME")?,
        };
        let comp_level = match get("COMPRESSION_LEVEL") {
            Some(v) => ZstdLevel::new(parse("COMPRESSION_LEVEL", &v)?)?,
            None => ZstdLevel::default(),
        };
        let retention = match get("DRIVE_RETENTION_COUNT") {
            None => None,
            Some(v) => match parse::<usize>("DRIVE_RETENTION_COUNT", &v)? {
                0 => {
                    return Err(Error::Config(
                        "DRIVE_RETENTION_COUNT must be positive".into(),
                    ));
                }
                n => Some(n),
            },
        };
        let listen_addr = get("LISTEN_ADDR").unwrap_or_else(|| DEFAULT_LISTEN_ADDR.into());

        Ok(Self {
            listen_addr: parse("LISTEN_ADDR", &listen_addr)?,
            db,
            comp_level,
            drive_creds: Secret::new(required("DRIVE_CREDS")?),
            drive_folder_id: required("DRIVE_FOLDER_ID")?,
            data_folder: get("DATA_FOLDER")
                .unwrap_or_else(|| DEFAULT_DATA_FOLDER.into())
                .into(),
            retention,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;

    use backup_pg::ZstdLevel;

    use super::Settings;

    const REQUIRED: [(&str, &str); 6] = [
        ("DB_HOST", "tracker-db"),
        ("DB_USERNAME", "tracker"),
        ("DB_PASSWORD", "secret"),
        ("DB_NAME", "tracker"),
        ("DRIVE_CREDS", "{}"),
        ("DRIVE_FOLDER_ID", "folder"),
    ];

    fn parse(extra: &[(&str, &str)], without: &str) -> backup_core::Result<Settings> {
        let map: HashMap<String, String> = REQUIRED
            .iter()
            .chain(extra)
            .filter(|(k, _)| *k != without)
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Settings::from_source(move |key| map.get(key).cloned())
    }

    #[test]
    fn test_defaults() {
        let s = parse(&[], "").unwrap();

        assert_eq!(s.db.port, 5432);
        assert_eq!(s.db.name, "tracker");
        assert_eq!(s.db.password.expose(), "secret");
        assert_eq!(s.comp_level, ZstdLevel::default());
        assert_eq!(s.data_folder, Path::new("data"));
        assert_eq!(s.retention, None);
        assert_eq!(s.listen_addr.to_string(), "0.0.0.0:50051");
    }

    #[test]
    fn test_overrides() {
        let s = parse(
            &[
                ("DB_PORT", "5433"),
                ("COMPRESSION_LEVEL", "9"),
                ("DRIVE_RETENTION_COUNT", "30"),
                ("LISTEN_ADDR", "127.0.0.1:6000"),
            ],
            "",
        )
        .unwrap();

        assert_eq!(s.db.port, 5433);
        assert_eq!(s.comp_level, ZstdLevel::new(9).unwrap());
        assert_eq!(s.retention, Some(30));
        assert_eq!(s.listen_addr.port(), 6000);
    }

    #[test]
    fn test_every_required_key_is_checked() {
        for (key, _) in REQUIRED {
            let err = parse(&[], key).unwrap_err();
            assert!(err.to_string().contains(key), "{err}");
        }
    }

    #[test]
    fn test_invalid_values() {
        for (key, value) in [
            ("DB_PORT", "-1"),
            ("DB_PORT", "70000"),
            ("COMPRESSION_LEVEL", "3.14"),
            ("COMPRESSION_LEVEL", "42"),
            ("DRIVE_RETENTION_COUNT", "0"),
            ("DRIVE_RETENTION_COUNT", "x"),
            ("LISTEN_ADDR", "nowhere"),
        ] {
            assert!(parse(&[(key, value)], "").is_err(), "{key}={value}");
        }
    }
}

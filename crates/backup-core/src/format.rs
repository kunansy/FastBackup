use std::ffi::OsStr;
use std::path::Path;

use chrono::{DateTime, Utc};

use crate::{Error, Result};

const PG_CUSTOM_EXTENSION: &str = "pgdump";
const LEGACY_EXTENSION: &str = "dump";
const NAME_TIME_FORMAT: &str = "%Y-%m-%d_%H:%M:%S";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupFormat {
    /// `pg_dump --format=custom`, restored with `pg_restore`.
    PgCustom,
    /// zstd-compressed JSON written by FastBackup <= 0.5.
    LegacyJsonZstd,
}

impl BackupFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PgCustom => "pg_custom",
            Self::LegacyJsonZstd => "legacy_json_zstd",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pg_custom" => Some(Self::PgCustom),
            "legacy_json_zstd" => Some(Self::LegacyJsonZstd),
            _ => None,
        }
    }
}

pub fn backup_file_name(db_name: &str, now: DateTime<Utc>) -> String {
    format!(
        "backup_{db_name}_{}.{PG_CUSTOM_EXTENSION}",
        now.format(NAME_TIME_FORMAT)
    )
}

/// The stored format property wins; files without it are classified by extension.
pub fn detect_format(file_name: &str, format_property: Option<&str>) -> Result<BackupFormat> {
    if let Some(value) = format_property {
        return BackupFormat::parse(value).ok_or_else(|| {
            Error::FailedPrecondition(format!("unknown backup format '{value}' of '{file_name}'"))
        });
    }

    match Path::new(file_name).extension().and_then(OsStr::to_str) {
        Some(PG_CUSTOM_EXTENSION) => Ok(BackupFormat::PgCustom),
        Some(LEGACY_EXTENSION) => Ok(BackupFormat::LegacyJsonZstd),
        _ => Err(Error::FailedPrecondition(format!(
            "could not detect backup format of '{file_name}'"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::{BackupFormat, backup_file_name, detect_format};

    #[test]
    fn test_backup_file_name_uses_clock_time_and_pgdump_extension() {
        let now = Utc.with_ymd_and_hms(2026, 11, 6, 12, 30, 5).unwrap();

        assert_eq!(
            backup_file_name("tracker", now),
            "backup_tracker_2026-11-06_12:30:05.pgdump"
        );
    }

    #[test]
    fn test_format_roundtrips_through_str() {
        for format in [BackupFormat::PgCustom, BackupFormat::LegacyJsonZstd] {
            assert_eq!(BackupFormat::parse(format.as_str()), Some(format));
        }
    }

    #[test]
    fn test_detect_format_prefers_property_over_extension() {
        let format = detect_format("backup_x.dump", Some("pg_custom")).unwrap();

        assert_eq!(format, BackupFormat::PgCustom);
    }

    #[test]
    fn test_detect_format_by_extension() {
        assert_eq!(
            detect_format("backup_tracker_2024-01-01_00:00:00.dump", None).unwrap(),
            BackupFormat::LegacyJsonZstd
        );
        assert_eq!(
            detect_format("backup_tracker_2026-01-01_00:00:00.pgdump", None).unwrap(),
            BackupFormat::PgCustom
        );
    }

    #[test]
    fn test_detect_format_rejects_unknown() {
        assert!(detect_format("notes.txt", None).is_err());
        assert!(detect_format("backup.pgdump", Some("tar")).is_err());
    }
}

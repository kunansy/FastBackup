use std::path::Path;

use chrono::{DateTime, Utc};

use crate::BackupFormat;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupKind {
    /// Created on request or by schedule.
    Regular,
    /// Safety copy taken right before a restore overwrote the database.
    PreRestore,
}

impl BackupKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Regular => "regular",
            Self::PreRestore => "pre_restore",
        }
    }

    /// Files without the property predate it and are regular backups.
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some("pre_restore") => Self::PreRestore,
            _ => Self::Regular,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupInfo {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub size_bytes: Option<u64>,
    pub md5: Option<String>,
    pub format: BackupFormat,
    pub kind: BackupKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_page_token: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct UploadRequest<'a> {
    pub path: &'a Path,
    pub name: &'a str,
    pub db_name: &'a str,
    pub format: BackupFormat,
    pub kind: BackupKind,
}

/// Regular pg_dump backups beyond the newest `keep`. Legacy JSON backups and
/// pre-restore copies are never pruned.
pub fn retention_candidates(backups: &[BackupInfo], keep: usize) -> Vec<&BackupInfo> {
    let mut prunable: Vec<&BackupInfo> = backups
        .iter()
        .filter(|b| b.kind == BackupKind::Regular && b.format == BackupFormat::PgCustom)
        .collect();
    prunable.sort_by_key(|b| std::cmp::Reverse(b.created_at));
    prunable.into_iter().skip(keep).collect()
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::{BackupInfo, BackupKind, retention_candidates};
    use crate::BackupFormat;

    fn backup(id: &str, day: u32, format: BackupFormat, kind: BackupKind) -> BackupInfo {
        BackupInfo {
            id: id.into(),
            name: format!("{id}.pgdump"),
            created_at: Utc.with_ymd_and_hms(2026, 1, day, 0, 0, 0).unwrap(),
            size_bytes: None,
            md5: None,
            format,
            kind,
        }
    }

    fn ids(items: Vec<&BackupInfo>) -> Vec<&str> {
        items.into_iter().map(|b| b.id.as_str()).collect()
    }

    #[test]
    fn test_kind_parse_defaults_to_regular() {
        assert_eq!(BackupKind::parse(None), BackupKind::Regular);
        assert_eq!(
            BackupKind::parse(Some("pre_restore")),
            BackupKind::PreRestore
        );
        assert_eq!(
            BackupKind::parse(Some(BackupKind::Regular.as_str())),
            BackupKind::Regular
        );
    }

    #[test]
    fn test_retention_keeps_newest_regular_backups() {
        let backups = vec![
            backup("d1", 1, BackupFormat::PgCustom, BackupKind::Regular),
            backup("d3", 3, BackupFormat::PgCustom, BackupKind::Regular),
            backup("d2", 2, BackupFormat::PgCustom, BackupKind::Regular),
            backup("d4", 4, BackupFormat::PgCustom, BackupKind::Regular),
        ];

        assert_eq!(ids(retention_candidates(&backups, 2)), ["d2", "d1"]);
    }

    #[test]
    fn test_retention_skips_legacy_and_pre_restore() {
        let backups = vec![
            backup(
                "legacy",
                1,
                BackupFormat::LegacyJsonZstd,
                BackupKind::Regular,
            ),
            backup("safety", 2, BackupFormat::PgCustom, BackupKind::PreRestore),
            backup("new", 3, BackupFormat::PgCustom, BackupKind::Regular),
        ];

        assert!(retention_candidates(&backups, 0) == vec![&backups[2]]);
    }

    #[test]
    fn test_retention_with_fewer_backups_than_keep() {
        let backups = vec![backup("d1", 1, BackupFormat::PgCustom, BackupKind::Regular)];

        assert!(retention_candidates(&backups, 5).is_empty());
    }
}

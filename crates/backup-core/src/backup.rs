use std::path::Path;

use chrono::{DateTime, Utc};

use crate::{BackupFormat, Error, Result, backup_file_name};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupKind {
    /// Created by CreateBackup.
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

    /// Files without the property predate it and are regular backups. An unknown
    /// value is an error, so retention never treats such a file as prunable.
    pub fn parse(value: Option<&str>) -> Result<Self> {
        match value {
            None | Some("regular") => Ok(Self::Regular),
            Some("pre_restore") => Ok(Self::PreRestore),
            Some(other) => Err(Error::FailedPrecondition(format!(
                "unknown backup kind '{other}'"
            ))),
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

/// A local dump to store; the name and format follow from the kind.
#[derive(Debug, Clone)]
pub struct UploadRequest<'a> {
    path: &'a Path,
    name: String,
    db_name: &'a str,
    format: BackupFormat,
    kind: BackupKind,
}

impl<'a> UploadRequest<'a> {
    pub fn pg_dump(path: &'a Path, db_name: &'a str, kind: BackupKind, now: DateTime<Utc>) -> Self {
        Self {
            path,
            name: backup_file_name(db_name, kind, now),
            db_name,
            format: BackupFormat::PgCustom,
            kind,
        }
    }

    pub fn path(&self) -> &Path {
        self.path
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn db_name(&self) -> &str {
        self.db_name
    }

    pub fn format(&self) -> BackupFormat {
        self.format
    }

    pub fn kind(&self) -> BackupKind {
        self.kind
    }
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
    use std::path::Path;

    use chrono::{TimeZone, Utc};

    use super::{BackupInfo, BackupKind, UploadRequest, retention_candidates};
    use crate::{BackupFormat, Error};

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
        assert_eq!(BackupKind::parse(None).unwrap(), BackupKind::Regular);
        for kind in [BackupKind::Regular, BackupKind::PreRestore] {
            assert_eq!(BackupKind::parse(Some(kind.as_str())).unwrap(), kind);
        }
    }

    #[test]
    fn test_kind_parse_rejects_unknown_value() {
        let err = BackupKind::parse(Some("manual")).unwrap_err();

        assert!(matches!(err, Error::FailedPrecondition(_)), "{err:?}");
    }

    #[test]
    fn test_upload_request_derives_name_and_format() {
        let now = Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap();

        let req =
            UploadRequest::pg_dump(Path::new("/tmp/x"), "tracker", BackupKind::PreRestore, now);

        assert_eq!(
            req.name(),
            "pre_restore_backup_tracker_2026-01-02_03:04:05.pgdump"
        );
        assert_eq!(req.format(), BackupFormat::PgCustom);
        assert_eq!(req.kind(), BackupKind::PreRestore);
        assert_eq!(req.db_name(), "tracker");
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

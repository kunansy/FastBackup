use std::collections::HashMap;

use backup_core::{BackupInfo, BackupKind, Error, Result, UploadRequest, detect_format};
use google_drive3::api::File;

use crate::query::{PROP_DB_NAME, PROP_FORMAT, PROP_KIND};

fn required<T>(value: Option<T>, field: &str) -> Result<T> {
    value.ok_or_else(|| Error::Storage(format!("Drive file has no '{field}'")))
}

pub(crate) fn to_backup_info(file: File) -> Result<BackupInfo> {
    let id = required(file.id, "id")?;
    let name = required(file.name, "name")?;
    let created_at = required(file.created_time, "createdTime")?;
    let props = file.app_properties.unwrap_or_default();

    let format = detect_format(&name, props.get(PROP_FORMAT).map(String::as_str))?;
    let size_bytes = file.size.and_then(|s| u64::try_from(s).ok());

    Ok(BackupInfo {
        id,
        name,
        created_at,
        size_bytes,
        md5: file.md5_checksum,
        format,
        kind: BackupKind::parse(props.get(PROP_KIND).map(String::as_str))?,
    })
}

pub(crate) fn is_in_folder(file: &File, folder_id: &str) -> bool {
    file.trashed != Some(true)
        && file
            .parents
            .as_ref()
            .is_some_and(|p| p.iter().any(|id| id == folder_id))
}

pub(crate) fn upload_metadata(req: &UploadRequest<'_>, folder_id: &str) -> File {
    let props = HashMap::from([
        (PROP_FORMAT.to_string(), req.format().as_str().to_string()),
        (PROP_KIND.to_string(), req.kind().as_str().to_string()),
        (PROP_DB_NAME.to_string(), req.db_name().to_string()),
    ]);

    File {
        name: Some(req.name().to_string()),
        parents: Some(vec![folder_id.to_string()]),
        mime_type: Some(mime::APPLICATION_OCTET_STREAM.to_string()),
        app_properties: Some(props),
        ..File::default()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;

    use backup_core::{BackupFormat, BackupKind, UploadRequest};
    use chrono::{TimeZone, Utc};
    use google_drive3::api::File;

    use super::{is_in_folder, to_backup_info, upload_metadata};

    fn file(name: &str, props: Option<HashMap<String, String>>) -> File {
        File {
            id: Some("id1".into()),
            name: Some(name.into()),
            created_time: Some(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()),
            size: Some(42),
            md5_checksum: Some("abc".into()),
            app_properties: props,
            parents: Some(vec!["folder".into()]),
            ..File::default()
        }
    }

    #[test]
    fn test_legacy_file_without_properties() {
        let info = to_backup_info(file("backup_tracker_2024-01-01_00:00:00.dump", None)).unwrap();

        assert_eq!(info.format, BackupFormat::LegacyJsonZstd);
        assert_eq!(info.kind, BackupKind::Regular);
        assert_eq!(info.size_bytes, Some(42));
        assert_eq!(info.md5.as_deref(), Some("abc"));
    }

    #[test]
    fn test_properties_define_format_and_kind() {
        let props = HashMap::from([
            ("format".to_string(), "pg_custom".to_string()),
            ("kind".to_string(), "pre_restore".to_string()),
        ]);

        let info = to_backup_info(file("anything", Some(props))).unwrap();

        assert_eq!(info.format, BackupFormat::PgCustom);
        assert_eq!(info.kind, BackupKind::PreRestore);
    }

    #[test]
    fn test_unknown_kind_is_an_error() {
        let props = HashMap::from([("kind".to_string(), "manual".to_string())]);

        assert!(to_backup_info(file("x.pgdump", Some(props))).is_err());
    }

    #[test]
    fn test_missing_id_is_an_error() {
        let mut f = file("x.pgdump", None);
        f.id = None;

        assert!(to_backup_info(f).is_err());
    }

    #[test]
    fn test_is_in_folder() {
        let f = file("x.pgdump", None);
        let mut trashed = f.clone();
        trashed.trashed = Some(true);

        assert!(is_in_folder(&f, "folder"));
        assert!(!is_in_folder(&f, "other"));
        assert!(!is_in_folder(&trashed, "folder"));
    }

    #[test]
    fn test_upload_metadata_round_trips_through_mapping() {
        let req = UploadRequest::pg_dump(
            Path::new("/tmp/x"),
            "tracker",
            BackupKind::PreRestore,
            Utc::now(),
        );

        let mut meta = upload_metadata(&req, "folder");
        assert_eq!(meta.parents, Some(vec!["folder".to_string()]));
        meta.id = Some("id".into());
        meta.created_time = Some(Utc::now());

        let info = to_backup_info(meta).unwrap();
        assert_eq!(info.format, BackupFormat::PgCustom);
        assert_eq!(info.kind, BackupKind::PreRestore);
    }
}

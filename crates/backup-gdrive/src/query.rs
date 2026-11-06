use backup_core::BackupKind;

pub(crate) const FILE_FIELDS: &str =
    "id,name,createdTime,size,md5Checksum,appProperties,parents,trashed";
pub(crate) const NEWEST_FIRST: &str = "createdTime desc";

pub(crate) const PROP_FORMAT: &str = "format";
pub(crate) const PROP_KIND: &str = "kind";
pub(crate) const PROP_DB_NAME: &str = "db_name";

pub(crate) fn list_fields() -> String {
    format!("nextPageToken,files({FILE_FIELDS})")
}

/// Escape a value for a single-quoted Drive query literal.
pub(crate) fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

const FOLDER_MIME: &str = "application/vnd.google-apps.folder";

pub(crate) fn folder_query(folder_id: &str) -> String {
    format!(
        "'{}' in parents and trashed = false and mimeType != '{FOLDER_MIME}'",
        escape(folder_id)
    )
}

pub(crate) fn latest_query(folder_id: &str) -> String {
    format!(
        "{} and not appProperties has {{ key='{PROP_KIND}' and value='{}' }}",
        folder_query(folder_id),
        BackupKind::PreRestore.as_str()
    )
}

#[cfg(test)]
mod tests {
    use super::{escape, folder_query, latest_query};

    #[test]
    fn test_escape_quotes_and_backslashes() {
        assert_eq!(escape(r"a'b\c"), r"a\'b\\c");
    }

    #[test]
    fn test_folder_query_escapes_id() {
        assert_eq!(
            folder_query("x' or name contains 'y"),
            r"'x\' or name contains \'y' in parents and trashed = false and mimeType != 'application/vnd.google-apps.folder'"
        );
    }

    #[test]
    fn test_latest_query_excludes_pre_restore_copies() {
        assert_eq!(
            latest_query("folder"),
            "'folder' in parents and trashed = false \
             and mimeType != 'application/vnd.google-apps.folder' \
             and not appProperties has { key='kind' and value='pre_restore' }"
        );
    }
}

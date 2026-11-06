use std::path::Path;
use std::time::{Duration, Instant};

use backup_core::{BackupInfo, Error, Page, Result, Storage, UploadRequest};
use google_drive3::api::Scope;
use http_body_util::BodyExt;
use md5::{Digest, Md5};
use tokio::io::AsyncWriteExt;

use crate::client::{Hub, build_hub};
use crate::errors::{CallError, timed};
use crate::mapping::{is_in_folder, to_backup_info, upload_metadata};
use crate::query::{FILE_FIELDS, NEWEST_FIRST, folder_query, latest_query, list_fields};
use crate::retry::{RetryPolicy, retry};

#[derive(Debug, Clone, Copy)]
pub struct DriveOptions {
    /// Limit for metadata calls and for receiving download headers.
    pub request_timeout: Duration,
    /// Limit for a whole upload.
    pub upload_timeout: Duration,
    /// Download fails when no bytes arrive for this long.
    pub read_idle_timeout: Duration,
    pub retry: RetryPolicy,
}

impl Default for DriveOptions {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(60),
            upload_timeout: Duration::from_secs(30 * 60),
            read_idle_timeout: Duration::from_secs(60),
            retry: RetryPolicy::default(),
        }
    }
}

/// Backups stored in one Google Drive folder.
pub struct GoogleDriveStorage {
    hub: Hub,
    folder_id: String,
    opts: DriveOptions,
}

impl GoogleDriveStorage {
    pub async fn connect(creds: &str, folder_id: String, opts: DriveOptions) -> Result<Self> {
        Ok(Self {
            hub: build_hub(creds).await?,
            folder_id,
            opts,
        })
    }

    #[cfg(test)]
    pub(crate) fn from_hub(hub: Hub, folder_id: &str, opts: DriveOptions) -> Self {
        Self {
            hub,
            folder_id: folder_id.to_string(),
            opts,
        }
    }

    async fn list_once(
        &self,
        query: &str,
        page_size: u32,
        page_token: Option<&str>,
    ) -> std::result::Result<Page<BackupInfo>, CallError> {
        let page_size = i32::try_from(page_size).unwrap_or(i32::MAX);
        let mut call = self
            .hub
            .files()
            .list()
            .add_scope(Scope::Full)
            .q(query)
            .order_by(NEWEST_FIRST)
            .page_size(page_size)
            .param("fields", &list_fields());
        if let Some(token) = page_token {
            call = call.page_token(token);
        }

        let (_, list) = timed("Listing backups", self.opts.request_timeout, call.doit()).await?;
        let items = list
            .files
            .unwrap_or_default()
            .into_iter()
            .map(to_backup_info)
            .collect::<Result<Vec<_>>>()
            .map_err(CallError::Permanent)?;

        Ok(Page {
            items,
            next_page_token: list.next_page_token,
        })
    }

    async fn download_once(
        &self,
        backup: &BackupInfo,
        dst: &Path,
    ) -> std::result::Result<u64, CallError> {
        let call = self
            .hub
            .files()
            .get(&backup.id)
            .add_scope(Scope::Full)
            .param("alt", "media")
            .acknowledge_abuse(true)
            .doit();
        let (resp, _) = timed("Downloading backup", self.opts.request_timeout, call).await?;

        let mut file = tokio::fs::File::create(dst)
            .await
            .map_err(|e| CallError::Permanent(Error::io("Could not create download file", e)))?;
        let mut body = resp.into_body();
        let mut hasher = Md5::new();
        let mut size = 0u64;
        let idle = self.opts.read_idle_timeout;

        loop {
            let frame = tokio::time::timeout(idle, body.frame())
                .await
                .map_err(|_| CallError::Transient(format!("No download data for {idle:?}")))?;
            let Some(frame) = frame else { break };
            let frame =
                frame.map_err(|e| CallError::Transient(format!("Download interrupted: {e}")))?;
            let Ok(data) = frame.into_data() else {
                continue;
            };

            hasher.update(&data);
            size += data.len() as u64;
            file.write_all(&data)
                .await
                .map_err(|e| CallError::Permanent(Error::io("Could not write download file", e)))?;
        }
        file.flush()
            .await
            .map_err(|e| CallError::Permanent(Error::io("Could not write download file", e)))?;

        if let Some(expected) = &backup.md5 {
            let actual = hex::encode(hasher.finalize());
            if !actual.eq_ignore_ascii_case(expected) {
                return Err(CallError::Transient(format!(
                    "Checksum mismatch for '{}': expected {expected}, got {actual}",
                    backup.name
                )));
            }
        }
        Ok(size)
    }
}

#[async_trait::async_trait]
impl Storage for GoogleDriveStorage {
    async fn upload(&self, req: UploadRequest<'_>) -> Result<BackupInfo> {
        log::info!("Uploading '{}'", req.name);
        let start = Instant::now();

        let file = std::fs::File::open(req.path)
            .map_err(|e| Error::io(format!("Could not open {:?}", req.path), e))?;
        let call = self
            .hub
            .files()
            .create(upload_metadata(&req, &self.folder_id))
            .add_scope(Scope::Full)
            .param("fields", FILE_FIELDS)
            .upload_resumable(file, mime::APPLICATION_OCTET_STREAM);

        // Not retried: a repeated create would leave a duplicate file.
        let (_, created) = timed("Uploading backup", self.opts.upload_timeout, call)
            .await
            .map_err(CallError::into_error)?;
        let info = to_backup_info(created)?;

        log::info!(
            "Uploaded '{}' as '{}' for {:?}",
            info.name,
            info.id,
            start.elapsed()
        );
        Ok(info)
    }

    async fn list(&self, page_size: u32, page_token: Option<&str>) -> Result<Page<BackupInfo>> {
        let query = folder_query(&self.folder_id);
        retry(self.opts.retry, "Listing backups", || {
            self.list_once(&query, page_size, page_token)
        })
        .await
    }

    async fn latest(&self) -> Result<Option<BackupInfo>> {
        let query = latest_query(&self.folder_id);
        let page = retry(self.opts.retry, "Getting latest backup", || {
            self.list_once(&query, 1, None)
        })
        .await?;
        Ok(page.items.into_iter().next())
    }

    async fn get(&self, id: &str) -> Result<BackupInfo> {
        let file = retry(self.opts.retry, "Getting backup", || async {
            let call = self
                .hub
                .files()
                .get(id)
                .add_scope(Scope::Full)
                .param("fields", FILE_FIELDS)
                .doit();
            timed("Getting backup", self.opts.request_timeout, call)
                .await
                .map(|(_, file)| file)
        })
        .await?;

        if !is_in_folder(&file, &self.folder_id) {
            return Err(Error::NotFound(format!("backup '{id}'")));
        }
        to_backup_info(file)
    }

    async fn download(&self, backup: &BackupInfo, dst: &Path) -> Result<()> {
        log::info!("Downloading '{}' ({})", backup.name, backup.id);
        let start = Instant::now();

        let size = retry(self.opts.retry, "Downloading backup", || {
            self.download_once(backup, dst)
        })
        .await?;

        log::info!(
            "Downloaded '{}', {size} bytes for {:?}",
            backup.name,
            start.elapsed()
        );
        Ok(())
    }

    async fn delete(&self, id: &str) -> Result<()> {
        let res = retry(self.opts.retry, "Deleting backup", || async {
            let call = self.hub.files().delete(id).add_scope(Scope::Full).doit();
            timed("Deleting backup", self.opts.request_timeout, call).await
        })
        .await;

        match res {
            Ok(_) | Err(Error::NotFound(_)) => {
                log::info!("Deleted backup '{id}'");
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use backup_core::{BackupFormat, BackupKind, Error, Storage, UploadRequest};
    use md5::{Digest, Md5};
    use serde_json::{Value, json};

    use super::{DriveOptions, GoogleDriveStorage};
    use crate::RetryPolicy;
    use crate::client::test_hub;
    use crate::fake::{FakeDrive, Reply};

    const FOLDER: &str = "folder1";

    fn opts(attempts: u32) -> DriveOptions {
        DriveOptions {
            request_timeout: Duration::from_secs(5),
            upload_timeout: Duration::from_secs(5),
            read_idle_timeout: Duration::from_secs(5),
            retry: RetryPolicy {
                attempts,
                base_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(2),
            },
        }
    }

    fn storage(fake: &FakeDrive, attempts: u32) -> GoogleDriveStorage {
        GoogleDriveStorage::from_hub(test_hub(&fake.base), FOLDER, opts(attempts))
    }

    fn md5_hex(data: &[u8]) -> String {
        hex::encode(Md5::digest(data))
    }

    fn file_json(id: &str, parent: &str, md5: &str) -> Value {
        json!({
            "id": id,
            "name": format!("backup_tracker_{id}.pgdump"),
            "createdTime": "2026-01-01T00:00:00Z",
            "size": "5",
            "md5Checksum": md5,
            "parents": [parent],
            "appProperties": {"format": "pg_custom", "kind": "regular"}
        })
    }

    fn drive_error(code: u16) -> Reply {
        Reply::json(
            code,
            json!({"error": {"code": code, "message": "boom", "errors": [{"reason": "x"}]}}),
        )
    }

    #[tokio::test]
    async fn test_list_retries_transient_error() {
        let fake = FakeDrive::start(vec![
            drive_error(503),
            Reply::json(
                200,
                json!({"files": [file_json("f1", FOLDER, "m")], "nextPageToken": "t2"}),
            ),
        ])
        .await;

        let page = storage(&fake, 3).list(10, Some("t1")).await.unwrap();

        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].format, BackupFormat::PgCustom);
        assert_eq!(page.next_page_token.as_deref(), Some("t2"));
        let requests = fake.requests();
        assert_eq!(requests.len(), 2);
        assert!(
            requests[1].contains("orderBy=createdTime"),
            "{}",
            requests[1]
        );
        assert!(requests[1].contains("pageToken=t1"), "{}", requests[1]);
        assert!(requests[1].contains("folder1"), "{}", requests[1]);
    }

    #[tokio::test]
    async fn test_latest_on_empty_folder() {
        let fake = FakeDrive::start(vec![Reply::json(200, json!({"files": []}))]).await;

        assert_eq!(storage(&fake, 1).latest().await.unwrap(), None);
        assert!(fake.requests()[0].contains("pre_restore"));
    }

    #[tokio::test]
    async fn test_get_rejects_file_outside_folder() {
        let fake = FakeDrive::start(vec![Reply::json(200, file_json("f1", "foreign", "m"))]).await;

        let err = storage(&fake, 1).get("f1").await.unwrap_err();

        assert!(matches!(err, Error::NotFound(_)), "{err:?}");
    }

    #[tokio::test]
    async fn test_get_maps_404_without_retry() {
        let fake = FakeDrive::start(vec![drive_error(404), drive_error(404)]).await;

        let err = storage(&fake, 3).get("missing").await.unwrap_err();

        assert!(matches!(err, Error::NotFound(_)), "{err:?}");
        assert_eq!(fake.requests().len(), 1);
    }

    #[tokio::test]
    async fn test_download_writes_content_and_checks_md5() {
        let content = b"pgdump-bytes".repeat(10_000);
        let fake = FakeDrive::start(vec![
            Reply::json(200, file_json("f1", FOLDER, &md5_hex(&content))),
            Reply::bytes(&content),
        ])
        .await;
        let drive = storage(&fake, 1);
        let dir = tempfile::tempdir().unwrap();
        let dst = dir.path().join("x.pgdump");

        let info = drive.get("f1").await.unwrap();
        drive.download(&info, &dst).await.unwrap();

        assert_eq!(std::fs::read(&dst).unwrap(), content);
        assert!(fake.requests()[1].contains("alt=media"));
    }

    #[tokio::test]
    async fn test_download_retries_on_checksum_mismatch() {
        let fake = FakeDrive::start(vec![
            Reply::json(200, file_json("f1", FOLDER, &md5_hex(b"good"))),
            Reply::bytes(b"corrupted"),
            Reply::bytes(b"good"),
        ])
        .await;
        let drive = storage(&fake, 2);
        let dir = tempfile::tempdir().unwrap();
        let dst = dir.path().join("x.pgdump");

        let info = drive.get("f1").await.unwrap();
        drive.download(&info, &dst).await.unwrap();

        assert_eq!(std::fs::read(&dst).unwrap(), b"good");
    }

    #[tokio::test]
    async fn test_download_fails_on_persistent_checksum_mismatch() {
        let fake = FakeDrive::start(vec![
            Reply::json(200, file_json("f1", FOLDER, &md5_hex(b"good"))),
            Reply::bytes(b"bad"),
            Reply::bytes(b"bad"),
        ])
        .await;
        let drive = storage(&fake, 2);
        let dir = tempfile::tempdir().unwrap();

        let info = drive.get("f1").await.unwrap();
        let err = drive
            .download(&info, &dir.path().join("x"))
            .await
            .unwrap_err();

        assert!(err.to_string().contains("Checksum mismatch"), "{err}");
    }

    #[tokio::test]
    async fn test_upload_resumable() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("x.pgdump");
        std::fs::write(&src, b"dump!").unwrap();
        let fake_session = |base: &str| format!("{base}/upload/session-1");

        let fake = FakeDrive::start_with(|base| {
            vec![
                Reply {
                    status: 200,
                    headers: vec![("Location", fake_session(base))],
                    body: Vec::new(),
                },
                Reply::json(200, file_json("new1", FOLDER, &md5_hex(b"dump!"))),
            ]
        })
        .await;

        let info = storage(&fake, 1)
            .upload(UploadRequest {
                path: &src,
                name: "backup_tracker_new1.pgdump",
                db_name: "tracker",
                format: BackupFormat::PgCustom,
                kind: BackupKind::Regular,
            })
            .await
            .unwrap();

        assert_eq!(info.id, "new1");
        let requests = fake.requests();
        assert!(
            requests[0].starts_with("POST /resumable/upload/drive/v3/files"),
            "{requests:?}"
        );
        assert!(requests[0].contains("uploadType=resumable"), "{requests:?}");
        assert!(requests[0].contains("md5Checksum"), "{requests:?}");
        assert_eq!(requests[1], "POST /upload/session-1", "{requests:?}");
    }

    #[tokio::test]
    async fn test_delete_treats_missing_file_as_deleted() {
        let fake = FakeDrive::start(vec![drive_error(404)]).await;

        storage(&fake, 1).delete("gone").await.unwrap();
    }
}

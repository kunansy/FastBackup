use std::future::Future;
use std::io::Cursor;
use std::time::{self, Duration};

use bytes::Bytes;
use google_drive3::DriveHub;
use google_drive3::api::{File, Scope};
use google_drive3::common::{Body, Response};
use google_drive3::hyper_util::client::legacy::{Client, connect::HttpConnector};
use google_drive3::hyper_util::rt::TokioExecutor;
use google_drive3::yup_oauth2::{
    ServiceAccountAuthenticator, client::CustomHyperClientBuilder, parse_service_account_key,
};
use http_body_util::BodyExt;
use hyper_rustls::HttpsConnector;

use backup_core::{Errors, Res, Storage, compression::decompress};

type Connector = HttpsConnector<HttpConnector>;
type Hub = DriveHub<Connector>;

// Split by hyper across all resolved addresses, so an unreachable IP is skipped.
const DRIVE_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DRIVE_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_FOLDER_NAME: &str = "tracker";

fn storage_error(context: &str, err: impl std::fmt::Display) -> Errors {
    Errors::StorageError(format!("{context}: {err}"))
}

fn build_connector() -> Res<Connector> {
    let mut http = HttpConnector::new();
    http.enforce_http(false);
    http.set_connect_timeout(Some(DRIVE_CONNECT_TIMEOUT));

    let connector = hyper_rustls::HttpsConnectorBuilder::new()
        .with_native_roots()
        .map_err(|e| storage_error("Could not load native TLS roots", e))?
        .https_or_http()
        .enable_http1()
        .enable_http2()
        .wrap_connector(http);
    Ok(connector)
}

async fn with_timeout<F, T>(operation: &str, limit: Duration, fut: F) -> Res<T>
where
    F: Future<Output = Res<T>>,
{
    tokio::time::timeout(limit, fut).await.map_err(|_| {
        let msg = format!("{operation} timed out after {limit:?}");
        Errors::StorageError(msg)
    })?
}

pub struct GoogleDrive {
    hub: Hub,
}

impl GoogleDrive {
    pub async fn connect(creds: &str) -> Res<Self> {
        let secret = parse_service_account_key(creds)
            .map_err(|e| storage_error("Could not parse drive creds", e))?;

        let auth_client = Client::builder(TokioExecutor::new()).build(build_connector()?);
        let auth = ServiceAccountAuthenticator::with_client(
            secret,
            CustomHyperClientBuilder::from(auth_client),
        )
        .build()
        .await
        .map_err(|e| storage_error("Could not build drive authenticator", e))?;

        let client = Client::builder(TokioExecutor::new()).build(build_connector()?);
        Ok(Self {
            hub: DriveHub::new(client, auth),
        })
    }

    async fn get_file_id(&self, file_name: &str) -> Res<String> {
        log::debug!("Getting file id: '{file_name}'");

        let q = format!("name = '{file_name}'");

        let (_, files) = with_timeout("Getting file id", DRIVE_REQUEST_TIMEOUT, async {
            self.hub
                .files()
                .list()
                .add_scope(Scope::Full)
                .param("fields", "files(id)")
                .q(&q)
                .doit()
                .await
                .map_err(|e| storage_error("Error requesting to Google Drive", e))
        })
        .await?;

        let mut files = files.files.ok_or_else(|| {
            Errors::StorageError("Could not get files, response body is empty".to_string())
        })?;

        match files.len() {
            0 => Err(Errors::StorageError(format!(
                "File '{file_name}' not found"
            ))),
            1 => {
                let file_id = files[0]
                    .id
                    .take()
                    .ok_or_else(|| Errors::StorageError("Field 'id' not found".to_string()))?;
                log::info!("File id = '{file_id}'");
                Ok(file_id)
            }
            len => Err(Errors::StorageError(format!(
                "There are {len} items found for '{file_name}'"
            ))),
        }
    }

    pub fn build_file(name: impl ToString, parents: Option<Vec<String>>) -> File {
        File {
            name: Some(name.to_string()),
            parents,
            ..File::default()
        }
    }

    async fn download_file(&self, file_id: &str) -> Res<Bytes> {
        log::info!("Downloading file id '{file_id}'");
        let start = time::Instant::now();

        let (resp, _): (Response, File) =
            with_timeout("Downloading file", DRIVE_REQUEST_TIMEOUT, async {
                self.hub
                    .files()
                    .get(file_id)
                    .add_scope(Scope::Full)
                    .param("alt", "media")
                    .acknowledge_abuse(true)
                    .doit()
                    .await
                    .map_err(|e| storage_error("Downloading failed", e))
            })
            .await?;

        let body: Body = resp.into_body();
        let file_content = with_timeout("Reading file body", DRIVE_REQUEST_TIMEOUT, async {
            body.collect()
                .await
                .map(|collected| collected.to_bytes())
                .map_err(|e| storage_error("Could not read response body", e))
        })
        .await?;

        log::info!("File got for {:?}", start.elapsed());
        Ok(file_content)
    }

    /// Get id and name of the newest file in the folder.
    pub async fn get_last_dump_id(&self, folder_id: &str) -> Res<(String, String)> {
        log::info!("Getting id of the last dump, folder id: '{folder_id}'");
        let start = time::Instant::now();

        let q = format!("'{folder_id}' in parents");
        let (_, files) = with_timeout("Getting last dump id", DRIVE_REQUEST_TIMEOUT, async {
            self.hub
                .files()
                .list()
                .add_scope(Scope::Full)
                .param("fields", "files(id,name,createdTime)")
                .q(&q)
                .doit()
                .await
                .map_err(|e| storage_error("Error requesting to Google Drive", e))
        })
        .await?;

        let newest = files
            .files
            .unwrap_or_default()
            .into_iter()
            .filter(|f| f.created_time.is_some())
            .max_by_key(|f| f.created_time)
            .ok_or_else(|| {
                Errors::StorageError("Could not get files, response body is empty".to_string())
            })?;

        let (Some(file_id), Some(file_name)) = (newest.id, newest.name) else {
            return Err(Errors::StorageError(
                "Fields 'id' and 'name' are required".to_string(),
            ));
        };
        log::info!(
            "Dump name '{file_name}', id '{file_id}' got for {:?}",
            start.elapsed()
        );

        Ok((file_id, file_name))
    }

    pub async fn resolve_folder_id(&self, folder_id: &Option<String>) -> Res<String> {
        match folder_id {
            Some(v) => Ok(v.clone()),
            None => self.get_file_id(DEFAULT_FOLDER_NAME).await,
        }
    }
}

#[async_trait::async_trait]
impl Storage for GoogleDrive {
    async fn upload(&self, buf: &[u8], filename: &str, folder_id: Option<String>) -> Res<String> {
        log::info!("Sending file '{filename}'");
        let start = time::Instant::now();

        let folder_id = self.resolve_folder_id(&folder_id).await?;
        let req = GoogleDrive::build_file(filename, Some(vec![folder_id]));
        let mime = "application/octet-stream"
            .parse()
            .map_err(|e| storage_error("Invalid mime type", e))?;

        let (_, file) = with_timeout("Uploading file", DRIVE_REQUEST_TIMEOUT, async {
            self.hub
                .files()
                .create(req)
                .add_scope(Scope::Full)
                .upload(Cursor::new(buf.to_vec()), mime)
                .await
                .map_err(|e| storage_error("Sending failed", e))
        })
        .await?;

        let file_id = file.id.unwrap_or_else(|| "undefined".to_string());
        log::info!("File id '{file_id}' sent for {:?}", start.elapsed());
        Ok(file_id)
    }

    async fn download(&self, folder_id: &str) -> Res<(Vec<u8>, String)> {
        let (file_id, file_name) = self.get_last_dump_id(folder_id).await?;

        let compressed_file = self.download_file(&file_id).await?;
        let file_content = decompress(&compressed_file)?;

        Ok((file_content, file_name))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use backup_core::Errors;

    use super::with_timeout;

    #[tokio::test]
    async fn test_with_timeout_returns_storage_error_when_future_hangs() {
        let res: backup_core::Res<()> = with_timeout(
            "Hanging op",
            Duration::from_millis(10),
            std::future::pending(),
        )
        .await;

        match res {
            Err(Errors::StorageError(msg)) => {
                assert!(msg.starts_with("Hanging op timed out"), "{msg}")
            }
            other => panic!("expected StorageError, {other:?} found"),
        }
    }

    #[tokio::test]
    async fn test_with_timeout_passes_result_through() {
        let res = with_timeout("Fast op", Duration::from_secs(1), async { Ok(42) }).await;

        assert_eq!(res.unwrap(), 42);
    }
}

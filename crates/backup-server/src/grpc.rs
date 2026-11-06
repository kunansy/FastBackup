use std::sync::Arc;

use backup_proto::backup_server::Backup;
use backup_proto::download_chunk::Payload;
use backup_proto::{
    BackupInfo, BackupRef, CreateBackupRequest, DownloadChunk, HealthcheckReply,
    HealthcheckRequest, ListBackupsReply, ListBackupsRequest, RestoreReply, RestoreRequest,
};
use tempfile::NamedTempFile;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use crate::convert::{backup_info, backup_target};
use crate::service::BackupService;
use crate::status::to_status;

const CHUNK_SIZE: usize = 1024 * 1024;
const STREAM_BUFFER: usize = 4;

pub struct GrpcBackup {
    service: Arc<BackupService>,
}

impl GrpcBackup {
    pub fn new(service: Arc<BackupService>) -> Self {
        Self { service }
    }
}

type ChunkSender = mpsc::Sender<Result<DownloadChunk, Status>>;

/// Stream `file` in chunks; the file is removed when the task ends.
async fn stream_file(file: NamedTempFile, tx: ChunkSender) {
    let mut reader = match tokio::fs::File::open(file.path()).await {
        Ok(f) => f,
        Err(e) => {
            let _ = tx
                .send(Err(Status::internal(format!(
                    "Could not open download: {e}"
                ))))
                .await;
            return;
        }
    };
    let mut buf = vec![0u8; CHUNK_SIZE];
    loop {
        let chunk = match reader.read(&mut buf).await {
            Ok(0) => return,
            Ok(n) => Ok(DownloadChunk {
                payload: Some(Payload::Data(buf[..n].to_vec())),
            }),
            Err(e) => Err(Status::internal(format!("Could not read download: {e}"))),
        };
        let failed = chunk.is_err();
        if tx.send(chunk).await.is_err() || failed {
            return;
        }
    }
}

#[tonic::async_trait]
impl Backup for GrpcBackup {
    type DownloadBackupStream = ReceiverStream<Result<DownloadChunk, Status>>;

    async fn create_backup(
        &self,
        _request: Request<CreateBackupRequest>,
    ) -> Result<Response<BackupInfo>, Status> {
        log::info!("Request to create a backup");
        let info = self
            .service
            .create_backup()
            .await
            .map_err(|e| to_status("Backup failed", e))?;
        Ok(Response::new(backup_info(&info)))
    }

    async fn list_backups(
        &self,
        request: Request<ListBackupsRequest>,
    ) -> Result<Response<ListBackupsReply>, Status> {
        let req = request.into_inner();
        let token = Some(req.page_token.as_str()).filter(|t| !t.is_empty());
        let page = self
            .service
            .list(req.page_size, token)
            .await
            .map_err(|e| to_status("Listing failed", e))?;

        Ok(Response::new(ListBackupsReply {
            backups: page.items.iter().map(backup_info).collect(),
            next_page_token: page.next_page_token.unwrap_or_default(),
        }))
    }

    async fn get_backup(
        &self,
        request: Request<BackupRef>,
    ) -> Result<Response<BackupInfo>, Status> {
        let target = backup_target(Some(request.into_inner()))?;
        let info = self
            .service
            .resolve(&target)
            .await
            .map_err(|e| to_status("Getting backup failed", e))?;
        Ok(Response::new(backup_info(&info)))
    }

    async fn download_backup(
        &self,
        request: Request<BackupRef>,
    ) -> Result<Response<Self::DownloadBackupStream>, Status> {
        let target = backup_target(Some(request.into_inner()))?;
        let (info, file) = self
            .service
            .download(&target)
            .await
            .map_err(|e| to_status("Download failed", e))?;
        log::info!("Streaming '{}'", info.name);

        let (tx, rx) = mpsc::channel(STREAM_BUFFER);
        let header = DownloadChunk {
            payload: Some(Payload::Info(backup_info(&info))),
        };
        tokio::spawn(async move {
            if tx.send(Ok(header)).await.is_ok() {
                stream_file(file, tx).await;
            }
        });
        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn restore(
        &self,
        request: Request<RestoreRequest>,
    ) -> Result<Response<RestoreReply>, Status> {
        let req = request.into_inner();
        let target = backup_target(req.backup)?;
        log::info!("Request to restore {target:?}");

        let outcome = self
            .service
            .restore(&target, &req.confirm_db_name)
            .await
            .map_err(|e| to_status("Restore failed", e))?;

        Ok(Response::new(RestoreReply {
            restored: Some(backup_info(&outcome.restored)),
            pre_restore_backup: Some(backup_info(&outcome.pre_restore)),
        }))
    }

    async fn healthcheck(
        &self,
        _request: Request<HealthcheckRequest>,
    ) -> Result<Response<HealthcheckReply>, Status> {
        let is_ok = self.service.healthcheck().await;
        Ok(Response::new(HealthcheckReply { is_ok }))
    }
}

#[cfg(test)]
mod tests {
    use backup_core::{BackupFormat, BackupKind};
    use backup_proto::backup_ref::Target;
    use backup_proto::backup_server::Backup;
    use backup_proto::download_chunk::Payload;
    use backup_proto::{BackupRef, CreateBackupRequest, RestoreRequest};
    use tokio_stream::StreamExt;
    use tonic::{Code, Request};

    use super::{CHUNK_SIZE, GrpcBackup};
    use crate::service::tests::{harness, info};

    #[tokio::test]
    async fn test_download_streams_info_then_chunks() {
        let h = harness();
        let content: Vec<u8> = (0..CHUNK_SIZE * 2 + 10).map(|i| (i % 251) as u8).collect();
        h.storage.put(
            info("big", 1, BackupFormat::PgCustom, BackupKind::Regular),
            &content,
        );
        let grpc = GrpcBackup::new(h.service.clone());

        let mut stream = grpc
            .download_backup(Request::new(BackupRef {
                target: Some(Target::Latest(true)),
            }))
            .await
            .unwrap()
            .into_inner();

        let mut got = Vec::new();
        let mut sizes = Vec::new();
        let first = stream.next().await.unwrap().unwrap();
        assert!(matches!(first.payload, Some(Payload::Info(ref i)) if i.id == "big"));
        while let Some(chunk) = stream.next().await {
            let Some(Payload::Data(data)) = chunk.unwrap().payload else {
                panic!("expected data");
            };
            sizes.push(data.len());
            got.extend(data);
        }
        assert_eq!(got, content);
        assert_eq!(sizes, [CHUNK_SIZE, CHUNK_SIZE, 10]);
    }

    #[tokio::test]
    async fn test_get_backup_latest_skips_pre_restore() {
        let h = harness();
        h.storage.put(
            info(
                "legacy",
                1,
                BackupFormat::LegacyJsonZstd,
                BackupKind::Regular,
            ),
            b"x",
        );
        h.storage.put(
            info("safety", 2, BackupFormat::PgCustom, BackupKind::PreRestore),
            b"x",
        );
        let grpc = GrpcBackup::new(h.service.clone());

        let got = grpc
            .get_backup(Request::new(BackupRef {
                target: Some(Target::Latest(true)),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(got.id, "legacy");
        assert_eq!(got.format(), backup_proto::BackupFormat::LegacyJsonZstd);
    }

    #[tokio::test]
    async fn test_download_missing_backup() {
        let h = harness();
        let grpc = GrpcBackup::new(h.service.clone());

        let err = grpc
            .download_backup(Request::new(BackupRef {
                target: Some(Target::Latest(true)),
            }))
            .await
            .unwrap_err();

        assert_eq!(err.code(), Code::NotFound);
    }

    #[tokio::test]
    async fn test_restore_wrong_db_name() {
        let h = harness();
        h.storage.put(
            info("b", 1, BackupFormat::PgCustom, BackupKind::Regular),
            b"x",
        );
        let grpc = GrpcBackup::new(h.service.clone());

        let err = grpc
            .restore(Request::new(RestoreRequest {
                backup: Some(BackupRef {
                    target: Some(Target::Latest(true)),
                }),
                confirm_db_name: "prod".into(),
            }))
            .await
            .unwrap_err();

        assert_eq!(err.code(), Code::FailedPrecondition);
    }

    #[tokio::test]
    async fn test_create_backup_returns_info() {
        let h = harness();
        let grpc = GrpcBackup::new(h.service.clone());

        let info = grpc
            .create_backup(Request::new(CreateBackupRequest {}))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(info.format(), backup_proto::BackupFormat::PgCustom);
        assert_eq!(info.kind(), backup_proto::BackupKind::Regular);
    }
}

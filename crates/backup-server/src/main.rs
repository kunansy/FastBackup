mod logger;
mod settings;
mod status;

use std::sync::Arc;

use tokio::signal::unix::{SignalKind, signal};
use tonic::{Request, Response, Status, transport::Server};

use backup_core::{
    BackupFormat, BackupKind, Clock, DbTarget, Dumper, Error, Secret, Storage, SystemClock,
    UploadRequest, backup_file_name, compression::decompress,
};
use backup_gdrive::{DriveOptions, GoogleDriveStorage};
use backup_pg::{PgTools, TokioCommandRunner};
use backup_proto::google_drive_server::{GoogleDrive as GoogleDriveService, GoogleDriveServer};
use backup_proto::{BackupReply, DbRequest, DownloadReply, Empty, HealthcheckReply};
use settings::Settings;
use status::to_status;

const LISTEN_ADDR: &str = "0.0.0.0:50051";

fn db_target(req: DbRequest) -> Result<DbTarget, Status> {
    let port = u16::try_from(req.db_port)
        .map_err(|_| Status::invalid_argument(format!("invalid db_port {}", req.db_port)))?;
    Ok(DbTarget {
        host: req.db_host,
        port,
        user: req.db_username,
        password: Secret::new(req.db_password),
        name: req.db_name,
    })
}

struct Backup {
    cfg: Arc<Settings>,
    clock: Arc<dyn Clock>,
    storage: Arc<dyn Storage>,
}

impl Backup {
    fn pg_tools(&self, target: DbTarget) -> PgTools {
        PgTools::new(target, self.cfg.comp_level, Arc::new(TokioCommandRunner))
    }

    fn temp_file(&self, suffix: &str) -> Result<tempfile::NamedTempFile, Error> {
        tempfile::Builder::new()
            .prefix("backup_")
            .suffix(suffix)
            .tempfile_in(&self.cfg.data_folder)
            .map_err(|e| Error::io("Could not create temporary file", e))
    }

    async fn create_backup(&self, pg: &PgTools) -> Result<String, Error> {
        let file = self.temp_file(".pgdump")?;
        pg.dump(file.path()).await?;

        let name = backup_file_name(pg.db_name(), self.clock.now());
        let info = self
            .storage
            .upload(UploadRequest {
                path: file.path(),
                name: &name,
                db_name: pg.db_name(),
                format: BackupFormat::PgCustom,
                kind: BackupKind::Regular,
            })
            .await?;
        Ok(info.id)
    }

    async fn download_latest_legacy(&self) -> Result<Vec<u8>, Error> {
        let latest = self
            .storage
            .latest()
            .await?
            .ok_or_else(|| Error::NotFound("no backups".into()))?;
        if latest.format != BackupFormat::LegacyJsonZstd {
            return Err(Error::FailedPrecondition(format!(
                "latest backup '{}' is {}, this RPC serves only legacy JSON backups",
                latest.name,
                latest.format.as_str()
            )));
        }

        let file = self.temp_file(".dump")?;
        self.storage.download(&latest, file.path()).await?;
        let compressed = tokio::fs::read(file.path())
            .await
            .map_err(|e| Error::io("Could not read downloaded file", e))?;
        decompress(&compressed)
    }
}

#[tonic::async_trait]
impl GoogleDriveService for Backup {
    async fn backup(&self, request: Request<DbRequest>) -> Result<Response<BackupReply>, Status> {
        log::info!("Request to backup");
        let pg = self.pg_tools(db_target(request.into_inner())?);

        let file_id = self
            .create_backup(&pg)
            .await
            .map_err(|e| to_status("Backup failed", e))?;

        Ok(Response::new(BackupReply { file_id }))
    }

    async fn download_latest_backup(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<DownloadReply>, Status> {
        let file_content = self
            .download_latest_legacy()
            .await
            .map_err(|e| to_status("Download failed", e))?;

        Ok(Response::new(DownloadReply { file_content }))
    }

    async fn healthcheck(
        &self,
        request: Request<DbRequest>,
    ) -> Result<Response<HealthcheckReply>, Status> {
        let pg = self.pg_tools(db_target(request.into_inner())?);
        let is_ok = match pg.ping().await {
            Ok(()) => true,
            Err(e) => {
                log::warn!("Database healthcheck failed: {e}");
                false
            }
        };

        Ok(Response::new(HealthcheckReply { is_ok }))
    }
}

async fn shutdown_signal() {
    let (Ok(mut term), Ok(mut quit)) =
        (signal(SignalKind::terminate()), signal(SignalKind::quit()))
    else {
        log::error!("Could not install signal handlers, waiting for ctrl-c only");
        let _ = tokio::signal::ctrl_c().await;
        return;
    };

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = term.recv() => {},
        _ = quit.recv() => {},
    }
    log::info!("Received shutdown signal, terminating");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    logger::init();

    let cfg = Arc::new(Settings::from_env()?);
    let addr = LISTEN_ADDR.parse()?;
    let storage = GoogleDriveStorage::connect(
        &cfg.drive_creds,
        cfg.drive_folder_id.clone(),
        DriveOptions::default(),
    )
    .await?;

    log::info!("Start gRPC server on {addr}");
    Server::builder()
        .add_service(GoogleDriveServer::new(Backup {
            cfg,
            clock: Arc::new(SystemClock),
            storage: Arc::new(storage),
        }))
        .serve_with_shutdown(addr, shutdown_signal())
        .await?;

    Ok(())
}

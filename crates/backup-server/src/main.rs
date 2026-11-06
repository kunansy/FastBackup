mod logger;
mod settings;

use std::sync::Arc;

use tokio::signal::unix::{SignalKind, signal};
use tonic::{Request, Response, Status, transport::Server};

use backup_core::{Clock, DbTarget, Dumper, Error, Secret, Storage, SystemClock, backup_file_name};
use backup_gdrive::GoogleDrive;
use backup_pg::{PgTools, TokioCommandRunner};
use backup_proto::google_drive_server::{GoogleDrive as GoogleDriveService, GoogleDriveServer};
use backup_proto::{BackupReply, DbRequest, DownloadReply, Empty, HealthcheckReply};
use settings::Settings;

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
}

impl Backup {
    fn pg_tools(&self, target: DbTarget) -> PgTools {
        PgTools::new(target, self.cfg.comp_level, Arc::new(TokioCommandRunner))
    }

    async fn connect_drive(&self) -> Result<(GoogleDrive, String), Status> {
        let drive = GoogleDrive::connect(&self.cfg.drive_creds)
            .await
            .map_err(|e| unavailable_error("Drive is unavailable", e))?;
        let folder_id = drive
            .resolve_folder_id(&self.cfg.drive_folder_id)
            .await
            .map_err(|e| unavailable_error("Drive is unavailable", e))?;
        Ok((drive, folder_id))
    }

    async fn dump_to_buffer(&self, pg: &PgTools) -> Result<Vec<u8>, Error> {
        let file = tempfile::Builder::new()
            .prefix("backup_")
            .suffix(".pgdump")
            .tempfile_in(&self.cfg.data_folder)
            .map_err(|e| Error::io("Could not create temporary dump file", e))?;

        pg.dump(file.path()).await?;

        tokio::fs::read(file.path())
            .await
            .map_err(|e| Error::io("Could not read dump file", e))
    }
}

#[tonic::async_trait]
impl GoogleDriveService for Backup {
    async fn backup(&self, request: Request<DbRequest>) -> Result<Response<BackupReply>, Status> {
        log::info!("Request to backup");
        let pg = self.pg_tools(db_target(request.into_inner())?);
        let filename = backup_file_name(pg.db_name(), self.clock.now());

        let (dump, drive) = tokio::join!(self.dump_to_buffer(&pg), self.connect_drive());
        let dump = dump.map_err(|e| internal_error("Dump failed", e))?;
        let (drive, folder_id) = drive?;

        let file_id = drive
            .upload(&dump, &filename, Some(folder_id))
            .await
            .map_err(|e| unavailable_error("Upload failed", e))?;

        Ok(Response::new(BackupReply { file_id }))
    }

    async fn download_latest_backup(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<DownloadReply>, Status> {
        let (drive, folder_id) = self.connect_drive().await?;
        let (file_content, _) = drive
            .download(&folder_id)
            .await
            .map_err(|e| unavailable_error("Download failed", e))?;

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

fn internal_error(context: &str, err: impl std::fmt::Display) -> Status {
    log::error!("{context}: {err}");
    Status::internal(format!("{context}: {err}"))
}

fn unavailable_error(context: &str, err: impl std::fmt::Display) -> Status {
    log::error!("{context}: {err}");
    Status::unavailable(format!("{context}: {err}"))
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

    log::info!("Start gRPC server on {addr}");
    Server::builder()
        .add_service(GoogleDriveServer::new(Backup {
            cfg,
            clock: Arc::new(SystemClock),
        }))
        .serve_with_shutdown(addr, shutdown_signal())
        .await?;

    Ok(())
}

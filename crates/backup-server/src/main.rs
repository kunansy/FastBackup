mod logger;
mod settings;

use std::sync::Arc;

use tokio::signal::unix::{SignalKind, signal};
use tonic::{Request, Response, Status, transport::Server};

use backup_core::{DbConfig, Storage};
use backup_gdrive::GoogleDrive;
use backup_proto::google_drive_server::{GoogleDrive as GoogleDriveService, GoogleDriveServer};
use backup_proto::{BackupReply, DbRequest, DownloadReply, Empty, HealthcheckReply};
use settings::Settings;

const LISTEN_ADDR: &str = "0.0.0.0:50051";

struct RequestDb(DbRequest);

impl DbConfig for RequestDb {
    fn db_host(&self) -> &str {
        &self.0.db_host
    }

    fn db_port(&self) -> u16 {
        self.0.db_port as u16
    }

    fn db_username(&self) -> &str {
        &self.0.db_username
    }

    fn db_password(&self) -> &str {
        &self.0.db_password
    }

    fn db_name(&self) -> &str {
        &self.0.db_name
    }
}

struct Backup {
    cfg: Arc<Settings>,
}

impl Backup {
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
}

#[tonic::async_trait]
impl GoogleDriveService for Backup {
    async fn backup(&self, request: Request<DbRequest>) -> Result<Response<BackupReply>, Status> {
        log::info!("Request to backup");
        let params = RequestDb(request.into_inner());
        let comp_level = self.cfg.comp_level;

        let (dump, drive) = tokio::join!(
            backup_pg::prepare_dump(&params, comp_level),
            self.connect_drive()
        );
        let (dump, filename) = dump.map_err(|e| internal_error("Dump failed", e))?;
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
        let params = RequestDb(request.into_inner());
        let is_ok = backup_pg::healthcheck(&params).await;

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
        .add_service(GoogleDriveServer::new(Backup { cfg }))
        .serve_with_shutdown(addr, shutdown_signal())
        .await?;

    Ok(())
}

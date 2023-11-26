use std::sync::Arc;
use std::thread;

use signal_hook::{consts::SIGHUP, iterator::Signals};
use tonic::{Request, Response, Status, transport::Server};

use backup::{DbRequest, BackupReply, DownloadReply, HealthcheckReply, Empty};
use backup::google_drive_server::{GoogleDrive, GoogleDriveServer};
use backuper::{db, DbConfig, google_drive, logger, settings::Settings, Storage};

pub mod backup {
    tonic::include_proto!("backup");
}

#[derive(Debug, Default)]
pub struct Backup {}

impl DbConfig for DbRequest {
    fn db_host(&self) -> &String {
        &self.db_host
    }

    fn db_port(&self) -> u16 {
        self.db_port as u16
    }

    fn db_username(&self) -> &String {
        &self.db_username
    }

    fn db_password(&self) -> &String {
        &self.db_password
    }

    fn db_name(&self) -> &String {
        &self.db_name
    }
}

// I ensure that the var could not be accessed
// from the different threads to read/modify it
static mut CFG: Option<Arc<Settings>> = None;

#[tonic::async_trait]
impl GoogleDrive for Backup {
    async fn backup(&self, request: Request<DbRequest>) -> Result<Response<BackupReply>, Status> {
        log::info!("Request to backup");
        let cfg = match unsafe { CFG.clone() } {
            None => {
                let err = Status::internal(&"Settings not loaded".to_string());
                return Err(err);
            },
            Some(cfg) => cfg
        };

        let dump_hdl = tokio::spawn(async {
            let cfg = unsafe {CFG.clone()}.unwrap();
            let params = request.into_inner();

            db::prepare_dump(&params, cfg.comp_level).await
        });
        let drive_hdl = tokio::spawn(async move {
            // TODO: cache folder_id
            google_drive::prepare_drive(&cfg.drive_creds, &cfg.drive_folder_id).await
        });

        // TODO: manage errors
        let (dump, filename) = dump_hdl.await.unwrap().unwrap();
        let (drive, folder_id) = drive_hdl.await.unwrap().unwrap();

        let file_id = drive.upload(&dump, &filename, Some(folder_id))
            .await
            .map_err(|e| Status::aborted(e.to_string()))?;

        Ok(Response::new(BackupReply { file_id }))
    }

    async fn download_latest_backup(&self, _request: Request<Empty>) -> Result<Response<DownloadReply>, Status> {
        let cfg = match unsafe { CFG.clone() } {
            None => {
                let err = Status::internal(&"Settings not loaded".to_string());
                return Err(err);
            },
            Some(cfg) => cfg
        };

        let (drive, folder_id) = google_drive::prepare_drive(&cfg.drive_creds, &cfg.drive_folder_id).await.unwrap();
        let (file_content, _) = drive.download(&folder_id).await.unwrap();

        Ok(Response::new(DownloadReply { file_content }))
    }

    async fn healthcheck(&self, request: Request<DbRequest>) -> Result<Response<HealthcheckReply>, Status> {
        let params = request.into_inner();
        let is_ok = db::healthcheck(&params).await;

        Ok(Response::new(HealthcheckReply { is_ok }))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut signals = Signals::new(&[SIGHUP])?;

    // reload the settings on SIGHUP
    thread::spawn(move || {
        for sig in signals.forever() {
            log::info!("Received signal '{:?}', reload the settings", sig);
            load_settings();
            logger::init();
        }
    });

    load_settings();
    logger::init();

    let addr = "0.0.0.0:50051".parse()?;
    let server = Backup::default();

    log::info!("Start gRPC server");
    Server::builder()
        .add_service(GoogleDriveServer::new(server))
        .serve(addr)
        .await?;

    Ok(())
}

fn load_settings() {
    Settings::load_env(&None);
    let cfg = Settings::parse().expect("Could not get settings");
    unsafe {
        CFG = Some(Arc::new(cfg));
    }
}

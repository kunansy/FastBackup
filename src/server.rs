use std::path::Path;
use std::sync::Arc;
use std::thread;

use tonic::{Request, Response, Status, transport::Server};
use signal_hook::{iterator::Signals, consts::SIGHUP};
use once_cell::sync::Lazy;

use backuper::{db, DbConfig};
use backuper::google_drive::DriveAuth;
use backuper::settings::Settings;
use backup::{BackupReply, BackupRequest};
use backup::google_drive_server::{GoogleDrive, GoogleDriveServer};
use crate::backup::{RestoreReply, RestoreRequest};

pub mod backup {
    tonic::include_proto!("backup");
}

#[derive(Debug, Default)]
pub struct Backup {}

impl DbConfig for BackupRequest {
    fn db_host(&self) -> &String {
        &self.db_host
    }

    fn db_port(&self) -> &String {
        &self.db_port
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

impl DbConfig for RestoreRequest {
    fn db_host(&self) -> &String {
        &self.db_host
    }

    fn db_port(&self) -> &String {
        &self.db_port
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

static mut CFG: Lazy<Arc<Settings>> = Lazy::new(|| {
    Settings::load_env();
    Arc::new(Settings::parse().unwrap())
});

#[tonic::async_trait]
impl GoogleDrive for Backup {
    async fn backup(&self, request: Request<BackupRequest>) -> Result<Response<BackupReply>, Status> {
        log::info!("Request to backup: {:?}", request);
        let cfg = unsafe {
            CFG.clone()
        };

        let params = request.into_inner();
        if !db::is_db_ready(&params) {
            return Err(Status::not_found("The database not ready"));
        }

        let path = db::dump(&params, &cfg.data_folder, &cfg.encrypt_pub_key_file)
            .map_err(|e| Status::internal(e.to_string()))?;
        let path = Path::new(&path);

        let drive = {
            let mut d = DriveAuth::new(&cfg.drive_creds);
            d.build_auth().await?;
            d.build_hub()
        };
        let file_id = backuper::send(&drive, path)
            .await
            .map_err(|e| Status::aborted(e.to_string()))?;

        Ok(Response::new(BackupReply { file_id }))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut signals = Signals::new(&[SIGHUP])?;

    // reload the settings on SIGHUP
    thread::spawn(move || {
        for sig in signals.forever() {
            log::info!("Received signal '{:?}', reload the settings", sig);

            unsafe {
                CFG = Lazy::<Arc<Settings>>::new(|| {
                    Settings::load_env();
                    Arc::new(Settings::parse().unwrap())
                });
            }
        }
    });

    let addr = "0.0.0.0:50051".parse()?;
    let server = Backup::default();

    Server::builder()
        .add_service(GoogleDriveServer::new(server))
        .serve(addr)
        .await?;

    Ok(())
}

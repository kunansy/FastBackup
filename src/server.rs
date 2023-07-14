use std::path::Path;
use std::sync::Arc;
use std::thread;

use once_cell::sync::Lazy;
use signal_hook::{consts::SIGHUP, iterator::Signals};
use tonic::{Request, Response, Status, transport::Server};

use backup::{BackupReply, BackupRequest, RestoreRequest};
use backup::google_drive_server::{GoogleDrive, GoogleDriveServer};
use backuper::{db, DbConfig};
use backuper::{google_drive::DriveAuth, settings::Settings};

pub mod backup {
    tonic::include_proto!("backup");
}

#[derive(Debug, Default)]
pub struct Backup {}

impl DbConfig for BackupRequest {
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

impl DbConfig for RestoreRequest {
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
static mut CFG: Lazy<Arc<Settings>> = Lazy::new(|| {
    Settings::load_env(&None);
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

        let pool = db::init_pool(&params).await
            .map_err(|e| {
                let msg = format!("Could not create DB pool: {}", e.to_string());
                Status::internal(&msg)
            })?;

        let arc_pool = Arc::new(pool);
        let path = db::dump(arc_pool.clone(), &cfg.data_folder, cfg.comp_level)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        let path = Path::new(&path);

        let drive = {
            let mut d = DriveAuth::new(&cfg.drive_creds);
            d.build_auth().await?;
            d.build_hub()
        };
        let file_id = backuper::send(&drive, path, None)
            .await
            .map_err(|e| Status::aborted(e.to_string()))?;

        if params.delete_after {
            db::delete_dump(&path)
                .map_err(|e| {
                    let msg = format!("Could not delete dump: {:?}", e);
                    Status::internal(&msg)
                })?;
        }

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
                    Settings::load_env(&None);
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

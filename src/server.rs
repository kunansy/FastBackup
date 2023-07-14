use std::path::Path;
use std::sync::Arc;
use std::thread;

use signal_hook::{consts::SIGHUP, iterator::Signals};
use tonic::{Request, Response, Status, transport::Server};

use backup::{BackupReply, BackupRequest, RestoreRequest};
use backup::google_drive_server::{GoogleDrive, GoogleDriveServer};
use backuper::{db, DbConfig, settings::Settings};

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
static mut CFG: Option<Arc<Settings>> = None;

#[tonic::async_trait]
impl GoogleDrive for Backup {
    async fn backup(&self, request: Request<BackupRequest>) -> Result<Response<BackupReply>, Status> {
        log::info!("Request to backup: {:?}", request);
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

            db::prepare_dump(&params, &cfg.data_folder, cfg.comp_level).await
        });
        let drive_hdl = tokio::spawn(async move {
            // TODO: cache folder_id
            db::prepare_drive(&cfg.drive_creds, &cfg.drive_folder_id).await
        });

        // TODO: manage errors
        let path = dump_hdl.await.unwrap().unwrap();
        let (drive, folder_id) = drive_hdl.await.unwrap().unwrap();
        let path = Path::new(&path);

        let file_id = backuper::send(&drive, path, Some(folder_id))
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
            load_settings();
        }
    });

    load_settings();

    let addr = "0.0.0.0:50051".parse()?;
    let server = Backup::default();

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
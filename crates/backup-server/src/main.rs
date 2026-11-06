mod convert;
mod grpc;
mod logger;
mod service;
mod settings;
mod status;

use std::sync::Arc;

use backup_core::SystemClock;
use backup_gdrive::{DriveOptions, GoogleDriveStorage};
use backup_pg::{PgTools, TokioCommandRunner};
use backup_proto::backup_server::BackupServer;
use tokio::signal::unix::{SignalKind, signal};
use tonic::transport::Server;

use grpc::GrpcBackup;
use service::{BackupService, ServiceDeps};
use settings::Settings;

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

    let cfg = Settings::from_env()?;
    let storage = GoogleDriveStorage::connect(
        cfg.drive_creds.expose(),
        cfg.drive_folder_id.clone(),
        DriveOptions::default(),
    )
    .await?;
    let pg = Arc::new(PgTools::new(
        cfg.db.clone(),
        cfg.comp_level,
        cfg.pg_timeout,
        Arc::new(TokioCommandRunner),
    ));

    let service = Arc::new(BackupService::new(ServiceDeps {
        dumper: pg.clone(),
        restorer: pg.clone(),
        health: pg,
        storage: Arc::new(storage),
        clock: Arc::new(SystemClock),
        data_folder: cfg.data_folder.clone(),
        retention: cfg.retention,
    }));

    log::info!(
        "Start gRPC server on {} for database '{}'",
        cfg.listen_addr,
        service.db_name()
    );
    Server::builder()
        .add_service(BackupServer::new(GrpcBackup::new(service)))
        .serve_with_shutdown(cfg.listen_addr, shutdown_signal())
        .await?;

    Ok(())
}

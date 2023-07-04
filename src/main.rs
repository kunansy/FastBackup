use std::{path::Path, sync::Arc};
use std::time;

use backuper::{db, google_drive::DriveAuth, logger, Res, settings::Settings};
use backuper::google_drive::GoogleDrive;

#[tokio::main]
async fn main() -> Res<()> {
    log::info!("Start app");
    let start = time::Instant::now();

    Settings::load_env();
    logger::init();

    let cfg = Settings::parse()?;

    let dump_hdl = tokio::spawn( async {
        let c = Settings::parse().unwrap();
        log::info!("Prepare dump");
        let d = prepare_dump(&c).await;
        log::info!("Dump prepared");
        d
    });
    let drive_hdl = tokio::spawn(async move {
        log::info!("Prepare drive");
        let d = prepare_drive(&cfg.drive_creds).await;
        log::info!("Drive prepared");
        d
    });

    let dp = dump_hdl.await??;
    let (drive, folder_id) = drive_hdl.await??;

    // let pool = db::init_pool(&cfg).await?;
    // let arc_pool = Arc::new(pool);
    //
    // let filename = db::dump(arc_pool, &cfg.data_folder, cfg.comp_level).await?;
    let filename = Path::new(&dp);
    //
    // // TODO: add two parallel threads:
    // //  1. Init pool and dump db;
    // //  2. Init drive, get "tracker" folder id, last dump id
    // let drive = {
    //     let mut d = DriveAuth::new(&cfg.drive_creds);
    //     d.build_auth().await?;
    //     d.build_hub()
    // };

    backuper::send(&drive, &filename, Some(folder_id)).await?;

    log::info!("App completed for {:?}", start.elapsed());
    Ok(())
}

async fn prepare_dump(cfg: &Settings) -> Res<String> {
    let pool = db::init_pool(cfg).await?;
    let arc_pool = Arc::new(pool);

    db::dump(arc_pool, &cfg.data_folder, cfg.comp_level).await
}

async fn prepare_drive(creds: &String) -> Res<(GoogleDrive, String)> {
    let drive = {
        let mut d = DriveAuth::new(creds);
        d.build_auth().await?;
        d.build_hub()
    };
    let folder_id = drive.get_file_id("tracker").await?;

    Ok((drive, folder_id))
}
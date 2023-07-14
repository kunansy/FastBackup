use std::path::Path;
use std::time;

use backuper::{db, logger, Res, settings::Settings};

#[tokio::main]
async fn main() -> Res<()> {
    log::info!("Start app");
    let start = time::Instant::now();

    Settings::load_env(&Some(".env"));
    logger::init();

    let cfg = Settings::parse()?;

    let drive_hdl = tokio::spawn(async move {
        // TODO: cache folder_id
        db::prepare_drive(&cfg.drive_creds, &cfg.drive_folder_id).await
    });
    let dump_hdl = tokio::spawn( async {
        let c = Settings::parse().unwrap();
        db::prepare_dump(&c, &c.data_folder, c.comp_level).await
    });

    let path = dump_hdl.await??;
    let (drive, folder_id) = drive_hdl.await??;

    let filename = Path::new(&path);

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
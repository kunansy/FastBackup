use std::path::Path;
use std::time;
use std::fs;

use backuper::{db, google_drive, logger, Res, settings::Settings, Storage};

#[tokio::main]
async fn main() -> Res<()> {
    log::info!("Start app");
    let start = time::Instant::now();

    Settings::load_env(&Some(".env"));
    logger::init();

    let cfg = Settings::parse()?;

    let drive_hdl = tokio::spawn(async move {
        // TODO: cache folder_id
        google_drive::prepare_drive(&cfg.drive_creds, &cfg.drive_folder_id).await
    });
    let dump_hdl = tokio::spawn(async {
        let c = Settings::parse().unwrap();
        db::prepare_dump(&c, c.comp_level).await
    });

    let (dump, filename) = dump_hdl.await??;
    let (drive, folder_id) = drive_hdl.await??;

    drive.upload(&dump, &filename, Some(folder_id)).await?;

    log::info!("App completed for {:?}", start.elapsed());
    Ok(())
}

async fn download(cfg: &Settings) -> Res<()> {
    let (drive, folder_id) = google_drive::prepare_drive(
        &cfg.drive_creds, &cfg.drive_folder_id).await?;

    let (content, file_name) = drive.download(&folder_id).await?;

    let path = Path::new(&file_name);
    println!("Downloaded {:?}", path);

    let dst = path.with_extension("json");
    fs::write(dst, content)?;

    Ok(())
}
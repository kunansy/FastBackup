use std::{path::Path, sync::Arc};
use std::time;

use backuper::{db, google_drive::DriveAuth, logger, Res, settings::Settings};

#[tokio::main]
async fn main() -> Res<()> {
    log::info!("Start app");
    let start = time::Instant::now();

    Settings::load_env();
    logger::init();

    let cfg = Settings::parse()?;

    let pool = db::init_pool(&cfg).await?;
    let arc_pool = Arc::new(pool);

    let filename = db::dump(arc_pool, &cfg.data_folder, cfg.comp_level).await?;
    let filename = Path::new(&filename);

    // TODO: add two parallel threads: 
    //  1. Init pool and dump db; 
    //  2. Init drive, get "tracker" folder id, last dump id
    let drive = {
        let mut d = DriveAuth::new(&cfg.drive_creds);
        d.build_auth().await?;
        d.build_hub()
    };

    backuper::send(&drive, &filename).await?;

    log::info!("App completed for {:?}", start.elapsed());
    Ok(())
}

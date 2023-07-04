use std::path::Path;
use std::sync::Arc;
use std::time;

use backuper::{db, errors::Errors, google_drive::DriveAuth, logger, settings::Settings};

#[tokio::main]
async fn main() -> Result<(), Errors> {
    log::info!("Start app");
    let start = time::Instant::now();

    Settings::load_env();
    logger::init();

    let cfg = Settings::parse()?;

    let pool = db::init_pool(&cfg).await?;
    let arc_pool = Arc::new(pool);

    let filename = db::dump(arc_pool, &cfg.data_folder, cfg.comp_level).await?;
    let filename = Path::new(&filename);

    let drive = {
        let mut d = DriveAuth::new(&cfg.drive_creds);
        d.build_auth().await?;
        d.build_hub()
    };

    backuper::send(&drive, &filename).await?;

    log::info!("App completed for {:?}", start.elapsed());
    Ok(())
}

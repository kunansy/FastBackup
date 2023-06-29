use std::path::Path;
use std::time;
use backuper::{settings::Settings, logger, errors::Errors, db, google_drive::DriveAuth};

#[tokio::main]
async fn main() -> Result<(), Errors> {
    log::info!("Start app");

    db::assert_programs_exist();
    let start = time::Instant::now();

    Settings::load_env();
    logger::init();

    let cfg = Settings::parse()?;
    db::assert_db_is_ready(&cfg);

    let filename = db::dump(&cfg)?;
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

use std::path::Path;
use std::time;
use backuper::{settings::Settings, logger, errors::Errors, backup, google_drive::GoogleDrive};

#[tokio::main]
async fn main() -> Result<(), Errors> {
    backup::assert_programs_exist();
    let start = time::Instant::now();

    Settings::load_env();
    logger::init();

    let cfg = Settings::parse()?;
    backup::assert_db_is_ready(&cfg);

    let filename = backup::dump(&cfg)?;
    let filename = Path::new(&filename);
    
    let mut drive = GoogleDrive::new(&cfg.drive_creds);
    drive.build_auth().await?;
    drive.build_hub();
    
    backuper::send(&drive, &filename).await?;

    log::info!("Backup completed for {:?}", start.elapsed());
    Ok(())
}

use std::path::Path;
use backuper::{settings::Settings, logger, errors::Errors, backup, google_drive::GoogleDrive};
use backuper::sender::Storage;

#[tokio::main]
async fn main() -> Result<(), Errors> {
    backup::assert_programs_exist();

    Settings::load_env();
    logger::init();

    let cfg = Settings::parse()?;

    let filename = backup::dump(&cfg)?;
    let filename = Path::new(&filename);
    let drive = GoogleDrive::new(&cfg.drive_creds);
    drive.send(&filename).await?;

    Ok(())
}

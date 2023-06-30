pub use errors::Errors;

type Res<T> = Result<T, Errors>;

#[async_trait::async_trait]
pub trait Storage {
    async fn upload(&self, path: &std::path::Path) -> Res<String>;

    async fn download(&self, file_id: &str, path: &std::path::Path) -> Res<()>;
}

pub async fn send(store: &impl Storage, path: &std::path::Path) -> Res<String> {
    store.upload(path).await
}

pub mod settings {
    use std::{fs, num::ParseIntError};

    use crate::{errors::Errors, Res};

    #[derive(Debug)]
    pub struct Settings {
        pub db_host: String,
        pub db_port: String,
        pub db_username: String,
        pub db_password: String,
        pub db_name: String,
        pub encrypt_pub_key_file: String,
        pub drive_creds: String,
        // dump backups to this folder
        pub data_folder: Option<String>,
    }

    impl Settings {
        pub fn parse() -> Res<Self> {
            log::debug!("Parse settings");

            let db_host = std::env::var("DB_HOST")?;
            let db_username = std::env::var("DB_USERNAME")?;
            let db_password = std::env::var("DB_PASSWORD")?;
            let db_name = std::env::var("DB_NAME")?;
            let encrypt_pub_key_file = std::env::var("ENCRYPT_PUB_KEY_FILE")?;
            let drive_creds = std::env::var("DRIVE_CREDS")?;
            let data_folder = std::env::var("DATA_FOLDER")
                .map_or(None, |v| {
                    assert!(!v.ends_with('/'), "DATA_FOLDER could not ends with '/'");
                    Some(v)
                });
            let db_port = std::env::var("DB_PORT")?
                .parse::<u32>()
                .map_err(|e: ParseIntError|
                    Errors::EnvError(format!("DB_PORT must be int: {}", e.to_string())))?;

            log::debug!("Settings parsed");
            Ok(Self {
                db_host,
                db_port: db_port.to_string(),
                db_username,
                db_password,
                db_name,
                encrypt_pub_key_file,
                drive_creds,
                data_folder,
            })
        }

        /// load .env file to env.
        ///
        /// # errors
        ///
        /// warn if it could not read file, don't panic.
        pub fn load_env() {
            let env = match fs::read_to_string(".env") {
                Ok(content) => content,
                Err(e) => {
                    log::warn!("error reading .env file: {}", e);
                    return;
                }
            };

            for line in env.lines() {
                if line.is_empty() {
                    continue;
                }
                let (name, value) = match line.split_once("=") {
                    Some(pair) => pair,
                    None => continue
                };
                // there might be spaces around the '=', so trim the strings
                std::env::set_var(name.trim(), value.trim());
            }
        }
    }
}

pub mod logger {
    use std::io::Write;

    use chrono::Local;
    use env_logger::Builder;

    pub fn init() {
        Builder::new()
            .format(|buf, record| {
                writeln!(buf,
                         "{}\t[{}] [{}:{}]\t{}",
                         record.level(),
                         Local::now().format("%Y-%m-%d %H:%M:%S.%f"),
                         record.target(),
                         record.line().unwrap_or(1),
                         record.args()
                )
            })
            .parse_default_env()
            .init();
    }
}

pub mod db {
    use std::{path::{Path, PathBuf}, process::{Command, Stdio}, time};

    use crate::{errors::Errors, Res, settings::Settings};

    pub fn dump(cfg: &Settings) -> Res<String> {
        log::info!("Start backupping");
        let start = time::Instant::now();
        let filename = create_filename(&cfg.db_name, &cfg.data_folder);

        let pg_dump = Command::new("pg_dump")
            .env("PGPASSWORD", &cfg.db_password)
            .args(["-h", &cfg.db_host])
            .args(["-p", &cfg.db_port])
            .args(["-U", &cfg.db_username])
            .arg("--data-only")
            .arg("--verbose")
            .arg("--inserts")
            .arg("--blobs")
            .arg("--column-inserts")
            .arg(&cfg.db_name)
            .stdout(Stdio::piped())
            .spawn()?;

        let gzip = Command::new("gzip")
            .args(["-c", "--best"])
            .stdin(Stdio::from(pg_dump.stdout.unwrap()))
            .stdout(Stdio::piped())
            .spawn()?;

        // TODO: let encryption switched off
        let openssl = Command::new("openssl")
            .arg("smime")
            .arg("-encrypt")
            .arg("-aes256")
            .arg("-binary")
            .args(["-outform", "DEM"])
            .args(["-out", &filename])
            .arg(&cfg.encrypt_pub_key_file)
            .stdin(Stdio::from(gzip.stdout.unwrap()))
            .spawn()?;

        match openssl.wait_with_output() {
            Ok(s) => {
                log::info!("Backup completed for {:?}, {:?}", start.elapsed(), s);
                Ok(filename)
            },
            Err(e) => {
                let msg = format!("Backup error: {}", e);
                log::error!("{}", msg);
                Err(Errors::DumpError(msg))
            }
        }
    }

    fn create_filename(db_name: &str, folder: &Option<String>) -> String {
        let prefix = match folder {
            Some(v) => format!("{}/", v),
            None => "".to_string()
        };
        format!("{}backup_{}_{}.enc", prefix, db_name,
                chrono::Utc::now().format("%Y-%m-%d_%H:%M:%S"))
    }

    /// All required programs must exist.
    ///
    /// # Panics
    /// Panic if a program not found
    pub fn assert_programs_exist() {
        for p in vec!["openssl", "pg_dump", "gzip", "pg_isready"] {
            if find_it(p).is_none() {
                panic!("'{}' not found", p);
            }
        }
    }

    fn find_it<P>(exe_name: P) -> Option<PathBuf>
        where P: AsRef<Path>
    {
        std::env::var_os("PATH").and_then(|paths| {
            std::env::split_paths(&paths).filter_map(|dir| {
                let full_path = dir.join(&exe_name);
                match full_path.is_file() {
                    true => Some(full_path),
                    false => None
                }
            }).next()
        })
    }

    pub fn assert_db_is_ready(cfg: &Settings) {
        log::debug!("Check the database is alive");

        let c = Command::new("pg_isready")
            .args(["--host", &cfg.db_host])
            .args(["--port", &cfg.db_port])
            .args(["--timeout", "10"])
            .args(["--username", &cfg.db_username])
            .status().unwrap()
            .success();
        assert!(c, "DB is not ready");

        log::debug!("Db is alive");
    }
}

pub mod google_drive {
    use std::{fs, io, path::Path, time};

    use google_drive3::{DriveHub, hyper::client::HttpConnector, hyper_rustls};
    use google_drive3::api::File;
    use google_drive3::hyper::{self, Body, body, Response};
    use google_drive3::hyper_rustls::HttpsConnector;
    use google_drive3::oauth2::{authenticator::Authenticator, parse_service_account_key, ServiceAccountAuthenticator as ServiceAuth, ServiceAccountKey};

    use crate::{errors::Errors, Res, Storage};

    type Hub = DriveHub<HttpsConnector<HttpConnector>>;

    pub struct DriveAuth {
        secret: Option<ServiceAccountKey>,
        auth: Option<Authenticator<HttpsConnector<HttpConnector>>>,
    }
    impl DriveAuth {
        pub fn new(creds: &String) -> Self {
            let secret = parse_service_account_key(creds)
                .expect("Could not parse creds");

            DriveAuth { secret: Some(secret), auth: None }
        }

        pub async fn build_auth(&mut self) -> io::Result<()> {
            let secret = match self.secret.take() {
                Some(v) => v,
                None => panic!("Could not build auth from None")
            };

            let auth = ServiceAuth::builder(secret).build().await?;
            self.auth = Some(auth);
            Ok(())
        }

        pub fn build_hub(&mut self) -> GoogleDrive {
            let auth = match self.auth.take() {
                Some(v) => v,
                None => panic!("Could not build hub from None")
            };

            let connector = hyper_rustls::HttpsConnectorBuilder::new()
                .with_native_roots()
                .https_or_http()
                .enable_http1()
                .enable_http2()
                .build();

            let hub = DriveHub::new(
                hyper::Client::builder().build(connector),
                auth
            );
            GoogleDrive { hub }
        }
    }

    pub struct GoogleDrive {
        hub: Hub
    }
    impl GoogleDrive {
        pub async fn get_file_id(&self, file_name: &str) -> Res<String> {
            log::debug!("Getting file id: '{}'", file_name);

            let q = format!("name = '{}'", file_name);

            let (resp, files) = self.hub.files().list()
                .param("fields", "files(id)")
                .q(&q)
                .doit().await
                .map_err(|e| {
                    let msg = format!("Error requesting to Google Drive: {}", e.to_string());
                    Errors::StorageError(msg)
                })?;

            if !resp.status().is_success() {
                let msg = format!("Request error: {:?}, {:?}", resp.status(), resp.body());
                return Err(Errors::StorageError(msg));
            }

            let mut files = match files.files {
                Some(files) => files,
                None => {
                    let msg = "Could not get files, response body is empty".to_string();
                    return Err(Errors::StorageError(msg));
                }
            };

            match files.len() {
                0 => {
                    let msg = format!("File '{}' not found", file_name);
                    Err(Errors::StorageError(msg))
                },
                1 => {
                    // when length 0 there must be [0] element
                    if let Some(file_id) = files[0].id.take() {
                        log::info!("File id = '{}'", &file_id);
                        Ok(file_id)
                    } else {
                        Err(Errors::StorageError("Field 'id' not found".to_string()))
                    }
                },
                len @ _ => {
                    let msg = format!("There are {} items found for '{}'", len, file_name);
                    Err(Errors::StorageError(msg))
                }
            }
        }

        pub fn build_file(name: &str, parents: Option<Vec<String>>) -> File {
            let mut file = File::default();
            file.name = Some(name.to_string());
            file.parents = parents;
            file
        }

        pub async fn upload_file(&self, req: File, src_file: fs::File) -> Res<(Response<Body>, File)> {
            self.hub.files()
                .create(req)
                .upload(src_file, "application/octet-stream".parse().unwrap())
                .await
                .map_err(|e| {
                    let msg = format!("Sending failed: {:?}", e);
                    Errors::StorageError(msg)
                })
        }

        pub async fn download_file(&self, file_id: &str, path: &Path) -> Res<()> {
            log::info!("Downloading file id '{}' to {:?}", file_id, path);
            let start = time::Instant::now();

            let (resp, _) = self.hub
                .files()
                .get(file_id)
                .add_scope("https://www.googleapis.com/auth/drive")
                .param("alt", "media")
                .acknowledge_abuse(true)
                .doit()
                .await
                .map_err(|e| {
                    let msg = format!("Downloading failed: {:?}", e);
                    Errors::StorageError(msg)
                })?;

            if !resp.status().is_success() {
                let msg = format!("Downloading failed: {}, {:?}", resp.status(), resp.body());
                return Err(Errors::StorageError(msg));
            }

            let d = body::to_bytes(resp.into_body()).await.map_err(|e| {
                let msg = format!("Could not convert resp body to bytes: {:?}", e);
                Errors::StorageError(msg)
            })?;

            fs::write(path, d)?;

            log::info!("File got for {:?}", start.elapsed());
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl Storage for GoogleDrive {
        async fn upload(&self, path: &Path) -> Res<String> {
            log::info!("Sending file {:?}", path);
            let start = time::Instant::now();

            assert!(path.exists(), "File {:?} not found", path);
            // here we know that the file exists,
            // there might be permission error
            let src_file = fs::File::open(path).map_err(|e| {
                Errors::StorageError(format!("Error opening file '{:?}': {}", path, e))
            })?;

            let req = {
                let folder_id = self.get_file_id("tracker").await?;
                // path must be convertable to str
                GoogleDrive::build_file(path.to_str().unwrap(), Some(vec![folder_id]))
            };

            let (resp, file) = self.upload_file(req, src_file).await?;

            if !resp.status().is_success() {
                let msg = format!("Sending failed: {}, {:?}", resp.status(), resp.body());
                return Err(Errors::StorageError(msg));
            }

            let file_id = file.id.unwrap_or("undefined".to_string());
            log::info!("File id '{}' sent for {:?}", file_id, start.elapsed());
            Ok(file_id)
        }

        async fn download(&self, file_name: &str, path: &Path) -> Res<()> {
            // TODO: get the last backup file
            let file_id = self.get_file_id(file_name).await?;
            self.download_file(&file_id, path).await?;

            Ok(())
        }
    }
}

pub mod errors {
    #[derive(thiserror::Error, Debug, Clone)]
    pub enum Errors {
        #[error("Could not init with env: {0}")]
        EnvError(String),
        #[error("Could not dump the database")]
        DumpError(String),
        #[error("Error with the storage")]
        StorageError(String)
    }

    impl From<std::env::VarError> for Errors {
        fn from(value: std::env::VarError) -> Self {
            Self::EnvError(value.to_string())
        }
    }

    impl From<std::io::Error> for Errors {
        fn from(value: std::io::Error) -> Self {
            Self::DumpError(value.to_string())
        }
    }
}


#[cfg(test)]
mod test_settings {

    use std::path::Path;
    use crate::settings::Settings;

    #[test]
    fn test_parse_with_empty_env() {
        for (key, _) in std::env::vars() {
            std::env::remove_var(key);
        }
        // TODO: check not found msg
        assert!(Settings::parse().is_err());
    }

    #[test]
    fn test_parse_ok() {
        assert!(Path::new(".env").exists(), ".env should exist");
        Settings::load_env();

        assert!(Settings::parse().is_ok());
    }

    #[test]
    fn test_parse_without_a_var() {
        assert!(Path::new(".env").exists(), ".env should exist");
        Settings::load_env();

        std::env::remove_var("DB_NAME");
        assert!(std::env::var("DB_NAME").is_err());

        // TODO: check not found msg
        assert!(Settings::parse().is_err());
    }

    #[test]
    fn test_parse_wrong_data_folder() {
        assert!(Path::new(".env").exists(), ".env should exist");
        Settings::load_env();

        std::env::set_var("DATA_FOLDER", "test/");

        // TODO: check parse error msg
        assert!(Settings::parse().is_err());
    }

    #[test]
    fn test_parse_not_int_port() {
        assert!(Path::new(".env").exists(), ".env should exist");
        Settings::load_env();

        std::env::set_var("DB_PORT", "3.1415926535");

        // TODO: check parse error msg
        assert!(Settings::parse().is_err());
    }

    #[test]
    fn test_parse_negative_port() {
        assert!(Path::new(".env").exists(), ".env should exist");
        Settings::load_env();

        std::env::set_var("DB_PORT", "-42");

        // TODO: check parse error msg
        assert!(Settings::parse().is_err());
    }

    #[test]
    fn test_parse_zero_port() {
        assert!(Path::new(".env").exists(), ".env should exist");
        Settings::load_env();

        std::env::set_var("DB_PORT", "0");

        // TODO: check parse error msg
        assert!(Settings::parse().is_err());
    }

    #[test]
    fn test_load_env() {
        Settings::load_env();
    }
}
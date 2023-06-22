pub mod settings {
    use std::fs;
    use std::num::ParseIntError;
    use crate::errors::Errors;

    #[derive(Debug)]
    pub struct Settings {
        pub db_host: String,
        pub db_port: String,
        pub db_username: String,
        pub db_password: String,
        pub db_name: String,
        pub encrypt_pub_key_file: String,
        pub drive_creds: String,
    }

    impl Settings {
        pub fn parse() -> Result<Self, Errors> {
            log::debug!("Parse settings");

            let db_host = std::env::var("DB_HOST")?;
            let db_username = std::env::var("DB_USERNAME")?;
            let db_password = std::env::var("DB_PASSWORD")?;
            let db_name = std::env::var("DB_NAME")?;
            let encrypt_pub_key_file = std::env::var("ENCRYPT_PUB_KEY_FILE")?;
            let drive_creds = std::env::var("DRIVE_CREDS")?;
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
                drive_creds
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
                let (name, value) = line.split_once("=").unwrap();
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

pub mod backup {
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::time;
    use crate::errors::Errors;
    use crate::settings::Settings;

    pub fn dump(cfg: &Settings) -> Result<String, Errors> {
        log::info!("Start backupping");
        let start = time::Instant::now();
        let filename = create_filename(&cfg.db_name);

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

    fn create_filename(db_name: &str) -> String {
        format!("backup_{}_{}.enc", db_name,
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

pub mod sender {
    use std::path::Path;
    use async_trait::async_trait;
    use crate::errors::Errors;

    type Res = Result<String, Errors>;

    #[async_trait]
    pub trait Storage {
        async fn send(&self, path: &Path) -> Res;
    }

    pub async fn send(store: &impl Storage, path: &Path) -> Res {
        store.send(path).await
    }
}

pub mod google_drive {
    use std::path::Path;
    use std::{fs, io, time};
    use google_drive3::{DriveHub, hyper::client::HttpConnector, hyper_rustls};
    use google_drive3::oauth2::{ServiceAccountKey, self, authenticator::Authenticator};
    use async_trait::async_trait;
    use google_drive3::api::File;
    use google_drive3::hyper;
    use google_drive3::hyper_rustls::HttpsConnector;
    use crate::errors::Errors;
    use crate::sender::Storage;

    type Hub = DriveHub<HttpsConnector<HttpConnector>>;

    pub struct GoogleDrive {
        secret: ServiceAccountKey,
    }

    impl GoogleDrive {
        pub fn new(creds: &String) -> Self {
            let secret = oauth2::parse_service_account_key(creds)
                .expect("Could not parse creds");
            GoogleDrive { secret }
        }

        async fn build_auth(&self) -> io::Result<Authenticator<HttpsConnector<HttpConnector>>> {
            oauth2::ServiceAccountAuthenticator::builder(
                self.secret.clone()).build().await
        }

        pub async fn build_hub(&self) -> io::Result<Hub> {
            let auth = self.build_auth().await?;
            let connector = hyper_rustls::HttpsConnectorBuilder::new()
                .with_native_roots()
                .https_or_http()
                .enable_http1()
                .enable_http2()
                .build();

            Ok(DriveHub::new(hyper::Client::builder().build(connector), auth))
        }

        pub async fn get_file_id(hub: &Hub, file_name: &str) -> Result<String, Errors> {
            log::debug!("Getting file id: '{}'", file_name);
            let q = format!("name = '{}'", file_name);

            let (resp, files) = hub.files().list()
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
            if files.files.is_none() {
                let msg = "Could not get files, response body is empty".to_string();
                return Err(Errors::StorageError(msg));
            }

            let mut files = files.files.unwrap();
            match files.len() {
                0 => {
                    let msg = format!("File '{}' not found", file_name);
                    Err(Errors::StorageError(msg))
                },
                1 => {
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
    }

    #[async_trait]
    impl Storage for GoogleDrive {
        async fn send(&self, path: &Path) -> Result<String, Errors> {
            assert!(path.exists(), "File {:?} not found", path);

            log::info!("Sending file {:?}", path);
            let start = time::Instant::now();

            let hub = self.build_hub().await?;

            let req = {
                let mut file = File::default();
                let folder_id = GoogleDrive::get_file_id(&hub, "tracker").await?;

                file.name = Some(path.to_str().unwrap().to_string());
                file.parents = Some(vec![folder_id]);
                file
            };

            let (resp, file) = hub.files().create(req)
                .upload(fs::File::open(path).unwrap(),
                        "application/octet-stream".parse().unwrap())
                .await
                .map_err(|e| {
                    let msg = format!("Upload failed: {:?}", e);
                    Errors::StorageError(msg)
                })?;

            if !resp.status().is_success() {
                let msg = format!("Error uploading file: {}, {:?}",
                                  resp.status(), resp.body());
                return Err(Errors::StorageError(msg));
            }

            let file_id = file.id.unwrap_or("undefined".to_string());
            log::info!("File id '{}' sent for {:?}", file_id, start.elapsed());
            Ok(file_id)
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
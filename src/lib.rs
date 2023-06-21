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
        pub encrypt_pub_key_file: String
    }

    impl Settings {
        pub fn parse() -> Result<Self, Errors> {
            log::debug!("Parse settings");

            let db_host = std::env::var("DB_HOST")?;
            let db_username = std::env::var("DB_USERNAME")?;
            let db_password = std::env::var("DB_PASSWORD")?;
            let db_name = std::env::var("DB_NAME")?;
            let encrypt_pub_key_file = std::env::var("ENCRYPT_PUB_KEY_FILE")?;
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
                encrypt_pub_key_file
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

pub mod db {
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::time;
    use crate::errors::Errors;
    use crate::settings::Settings;

    fn create_filename(db_name: &str) -> String {
        format!("backup_{}_{}.enc", db_name,
                chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S"))
    }

    pub fn dump(cfg: &Settings) -> Result<(), Errors>{
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
        let mut openssl = Command::new("openssl")
            .arg("smime")
            .arg("-encrypt")
            .arg("-aes256")
            .arg("-binary")
            .args(["-outform", "DEM"])
            .args(["-out", &filename])
            .arg(&cfg.encrypt_pub_key_file)
            .stdin(Stdio::from(gzip.stdout.unwrap()))
            .spawn()?;

        match openssl.wait() {
            Ok(_) => {
                log::info!("Backup completed for {:?}", start.elapsed());
                Ok(())
            },
            Err(e) => {
                let msg = format!("Backup error: {}", e);
                log::error!("{}", msg);
                Err(Errors::DumpError(msg))
            }
        }
    }

    /// All required programs must exist.
    ///
    /// # Panics
    /// Panic if a program not found
    pub fn assert_programs_exist() {
        for p in vec!["openssl", "pg_dump", "gzip"] {
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
}

pub mod errors {
    #[derive(thiserror::Error, Debug, Clone)]
    pub enum Errors {
        #[error("Could not init with env: {0}")]
        EnvError(String),
        #[error("Could not dump the database")]
        DumpError(String)
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
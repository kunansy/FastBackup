pub mod settings {
    use std::{fs, time};
    use std::num::ParseIntError;
    use crate::errors::Errors;

    #[derive(Debug)]
    pub struct Settings {
        pub db_uri: String,
        pub db_timeout: time::Duration
    }

    impl Settings {
        pub fn parse() -> Result<Self, Errors> {
            log::debug!("Parse settings");

            let db_uri = std::env::var("DATABASE_URL")?;
            let timeout = std::env::var("DATABASE_TIMEOUT")?
                .parse()
                .map_err(|e: ParseIntError|
                    Errors::EnvError(format!("DATABASE_TIMEOUT must be int: {}", e.to_string())))?;
            let db_timeout = time::Duration::from_secs(timeout);

            log::debug!("Settings parsed");
            Ok(Self { db_uri, db_timeout })
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

pub mod errors {
    #[derive(thiserror::Error, Debug, Clone)]
    pub enum Errors {
        #[error("Could not init with env: {0}")]
        EnvError(String),
    }

    impl From<std::env::VarError> for Errors {
        fn from(value: std::env::VarError) -> Self {
            Self::EnvError(value.to_string())
        }
    }
}
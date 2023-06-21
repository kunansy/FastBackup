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
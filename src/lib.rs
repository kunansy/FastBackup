pub mod settings {
    use std::{fs, time};

    #[derive(Debug)]
    pub struct Settings {
        pub db_uri: String,
        pub db_timeout: time::Duration
    }

    impl Settings {
        pub fn parse() -> Self {
            log::debug!("Parse settings");

            let db_uri = std::env::var("DATABASE_URL")
                .expect("DATABASE_URL not found");
            let timeout = std::env::var("DB_TIMEOUT")
                .unwrap_or("10".to_string())
                .parse().expect("DATABASE_TIMEOUT should be int");
            let db_timeout = time::Duration::from_secs(timeout);

            log::debug!("Settings parsed");
            Self { db_uri, db_timeout }
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
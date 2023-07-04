pub use errors::Errors;

type Res<T> = Result<T, Errors>;

#[async_trait::async_trait]
pub trait Storage {
    async fn upload(&self, path: &std::path::Path) -> Res<String>;

    async fn download(&self, file_id: &str, path: &str) -> Res<String>;
}

pub trait Compression<T> {
    fn compress(&self, output: &T, level: i32) -> Res<()>;
}

pub trait Decompression<I> {
    fn decompress(input: &I) -> Res<Box<Self>>;
}

pub trait DbConfig {
    fn db_host(&self) -> &String;
    fn db_port(&self) -> &String;
    fn db_username(&self) -> &String;
    fn db_password(&self) -> &String;
    fn db_name(&self) -> &String;
}

pub async fn send(store: &impl Storage, path: &std::path::Path) -> Res<String> {
    store.upload(path).await
}

pub mod settings {
    use std::{fs, num::ParseIntError};

    use crate::{DbConfig, errors::Errors, Res};

    #[derive(Debug)]
    pub struct Settings {
        db_host: String,
        db_port: String,
        db_username: String,
        db_password: String,
        db_name: String,
        pub drive_creds: String,
        // dump backups to this folder
        pub data_folder: Option<String>,
        // compression level, default is 3
        pub comp_level: i32,
    }

    impl Settings {
        pub fn parse() -> Res<Self> {
            log::debug!("Parse settings");

            let db_host = std::env::var("DB_HOST")?;
            let db_username = std::env::var("DB_USERNAME")?;
            let db_password = std::env::var("DB_PASSWORD")?;
            let db_name = std::env::var("DB_NAME")?;
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
            let comp_level = std::env::var("COMPRESSION_LEVEL")
                .unwrap_or("3".to_string())
                .parse::<i32>()
                .map_err(|e: ParseIntError|
                    Errors::EnvError(format!("COMPRESSION_LEVEL must be int: {}", e.to_string())))?;

            assert!(comp_level <= 22, "Max compression level is 22, {} found", comp_level);

            log::debug!("Settings parsed");
            Ok(Self {
                db_host,
                db_port: db_port.to_string(),
                db_username,
                db_password,
                db_name,
                drive_creds,
                data_folder,
                comp_level,
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

    impl DbConfig for Settings {
        fn db_host(&self) -> &String {
            &self.db_host
        }

        fn db_port(&self) -> &String {
            &self.db_port
        }

        fn db_username(&self) -> &String {
            &self.db_username
        }

        fn db_password(&self) -> &String {
            &self.db_password
        }

        fn db_name(&self) -> &String {
            &self.db_name
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
    use std::{collections::HashMap, fmt::Display, sync::Arc};
    use std::time;

    use chrono::{NaiveDate, NaiveDateTime};
    use serde_json::Value;
    use sqlx::{Column, PgPool, Pool, Postgres, query, Row, TypeInfo};
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgRow, PgSslMode, PgTypeKind};
    use sqlx::types::Uuid;
    use tokio::join;

    use crate::{Compression, DbConfig, ordered_map::OMap, Res};

    type RowDump = HashMap<String, Value>;
    type TableDump = Vec<RowDump>;
    pub type DBDump<'a> = OMap<&'a String, TableDump>;

    #[derive(sqlx::Type, Debug)]
    #[sqlx(type_name = "materialtypesenum", rename_all = "lowercase")]
    enum MaterialTypesEnum {
        Book,
        Article,
        Course,
        Lecture,
        Audiobook
    }

    impl ToString for MaterialTypesEnum {
        fn to_string(&self) -> String {
            let v = match self {
                MaterialTypesEnum::Book => "book",
                MaterialTypesEnum::Article => "article",
                MaterialTypesEnum::Course => "course",
                MaterialTypesEnum::Lecture => "lecture",
                MaterialTypesEnum::Audiobook => "audiobook",
            };
            v.to_string()
        }
    }

    pub async fn dump(pool: Arc<PgPool>,
                      data_folder: &Option<String>,
                      compression_level: i32) -> Res<String> {
        log::info!("Start dumping");
        let start = time::Instant::now();

        let db_name = pool
            .connect_options()
            .get_database()
            .unwrap_or("undefined");

        let (tables, table_refs) = join!(
            get_tables(&pool),
            get_table_refs(&pool)
        );
        let (tables, table_refs) = (tables?, table_refs?);
        let tables_order = define_tables_order(&tables, &table_refs);

        let json = dump_all(pool.clone(), tables_order).await?;
        let filename = create_filename(db_name, data_folder);

        json.compress(&filename, compression_level)?;

        log::info!("Dump completed for {:?}", start.elapsed());
        Ok(filename)
    }

    pub async fn init_pool<T>(cfg: &T) -> Res<Pool<Postgres>>
        where T: DbConfig
    {
        let conn = PgConnectOptions::new()
            .host(cfg.db_host())
            .port(cfg.db_port().parse().unwrap())
            .username(cfg.db_username())
            .password(cfg.db_password())
            .database(cfg.db_name())
            .ssl_mode(PgSslMode::Disable);

        PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(time::Duration::from_secs(5))
            .idle_timeout(time::Duration::from_secs(5))
            .connect_with(conn)
            .await
            .map_err(|e| e.into())
    }

    async fn get_tables(pool: &PgPool) -> Res<Vec<String>> {
        log::info!("Getting tables");

        let tables = sqlx::query(
            "SELECT tablename FROM pg_catalog.pg_tables \
            WHERE schemaname != 'pg_catalog' AND schemaname != 'information_schema';")
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|r| r.get("tablename"))
            .collect::<Vec<String>>();

        log::info!("{} tables got", tables.len());
        Ok(tables)
    }

    async fn get_table_refs(pool: &PgPool) -> Res<HashMap<String, String>> {
        log::info!("Getting table refs");

        let refs = sqlx::query(
            "SELECT
                tc.table_name,
                ccu.table_name AS foreign_table_name
            FROM
                information_schema.table_constraints tc
                JOIN information_schema.constraint_column_usage ccu
                  ON ccu.constraint_name = tc.constraint_name
                  AND ccu.table_schema = tc.table_schema
            WHERE tc.table_schema != 'pg_catalog' and tc.table_name != ccu.table_name;")
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|r| (r.get("table_name"), r.get("foreign_table_name")))
            .collect::<HashMap<String, String>>();

        log::info!("{} table refs got", refs.len());

        Ok(refs)
    }

    fn define_tables_order<'a>(tables: &'a Vec<String>,
                                   table_refs: &'a HashMap<String, String>) -> Vec<&'a String> {
        let mut weights = HashMap::new();
        // how many links to the table
        for table_name in table_refs.values() {
            *weights.entry(table_name).or_insert(0) += 1;
        }
        // there might be tables without links, add them
        for table_name in tables {
            weights.entry(table_name).or_insert(0);
        }

        let mut order = weights
            .into_iter()
            .map(|(k, v)| (k, v))
            .collect::<Vec<(&String, i32)>>();

        order.sort_by(|(_, v), (_, v2)| v.cmp(v2).reverse());

        order.into_iter()
            .map(|(k, _)| k)
            .collect::<Vec<&String>>()
    }

    async fn dump_all<'a>(pool: Arc<PgPool>,
                              tables: Vec<&'a String>) -> Res<DBDump<'a>> {
        // save order of the tables with OMap
        let mut table_dumps = OMap::with_capacity(tables.len());

        let tasks = tables
            .into_iter()
            .map(|table| {
                (table, tokio::spawn(dump_table(pool.clone(), table.clone())))
            });

        // run all tasks concurrently
        for (table, task) in tasks.into_iter() {
            let table_dump =  task.await??;
            table_dumps.insert(table, table_dump);
        }

        Ok(table_dumps)
    }

    #[inline]
    async fn dump_table<T>(pool: Arc<PgPool>, table_name: T) -> Res<TableDump>
        where T: Display
    {
        let res = query(&format!("SELECT * FROM {}", table_name))
            .fetch_all(&*pool)
            .await?
            .into_iter()
            .map(|row| dump_row(row))
            .collect::<Vec<RowDump>>();

        Ok(res)
    }

    #[inline]
    fn dump_row(row: PgRow) -> RowDump {
        let mut res = HashMap::with_capacity(row.columns().len());

        for column in row.columns() {
            let value = match column.type_info().name() {
                "INT4" => {
                    let v = row.get::<Option<i32>, _>(column.name());
                    match v {
                        Some(v) => Value::Number(v.into()),
                        None => Value::Null
                    }
                },
                "INT8" => {
                    let v = row.get::<Option<i64>, _>(column.name());
                    match v {
                        Some(v) => Value::Number(v.into()),
                        None => Value::Null
                    }
                },
                "VARCHAR" | "TEXT" => {
                    let v = row.get::<Option<String>, _>(column.name());
                    match v {
                        Some(v) => Value::String(v),
                        None => Value::Null
                    }
                },
                "UUID" => {
                    let v = row.get::<Option<Uuid>, _>(column.name());
                    match v {
                        Some(v) => Value::String(v.to_string()),
                        None => Value::Null
                    }
                },
                "BOOL" => {
                    let v = row.get::<Option<bool>, _>(column.name());
                    match v {
                        Some(v) => Value::Bool(v),
                        None => Value::Null
                    }
                },
                "TIMESTAMP" => {
                    let v = row.get::<Option<NaiveDateTime>, _>(column.name());
                    match v {
                        Some(v) => Value::String(v.to_string()),
                        None => Value::Null
                    }
                },
                "JSONB" | "JSON" => {
                    row.get::<Value, _>(column.name())
                },
                "DATE" => {
                    let v = row.get::<Option<NaiveDate>, _>(column.name());
                    match v {
                        Some(v) => Value::String(v.to_string()),
                        None => Value::Null
                    }
                },
                // TODO: how to match type without name??
                "materialtypesenum" => {
                    let v = row.get::<Option<MaterialTypesEnum>, _>(column.name());
                    match v {
                        Some(v) => {
                            Value::String(v.to_string())
                        }
                        None => Value::Null
                    }
                }
                _ => {
                    match column.type_info().kind() {
                        PgTypeKind::Enum(_) => {
                            panic!("Enum column: {:?}, could not process", column);
                        },
                        v @ _ => {
                            panic!("Not processed type: {:?}, {:?}", v, column.type_info().name())
                        }
                    }
                }
            };

            res.insert(column.name().to_string(), value);
        }
        res
    }

    fn create_filename(db_name: &str, folder: &Option<String>) -> String {
        let prefix = match folder {
            Some(v) => format!("{}/", v),
            None => "".to_string()
        };
        format!("{}backup_{}_{}.enc", prefix, db_name,
                chrono::Utc::now().format("%Y-%m-%d_%H:%M:%S"))
    }

    #[cfg(test)]
    mod test_db {
        use crate::db;

        #[test]
        fn test_create_filename_with_folder() {
            let f = db::create_filename("tdb", &Some("tf".to_string()));

            assert!(f.starts_with("tf/backup_tdb_"));
            assert!(f.ends_with(".enc"));
        }

        #[test]
        fn test_create_filename_without_folder() {
            let f = db::create_filename("tdb", &None);

            assert!(f.starts_with("backup_tdb_"));
            assert!(f.ends_with(".enc"));
        }
    }
}

pub mod ordered_map {
    use std::collections::HashMap;
    use std::fmt::Debug;
    use std::hash::Hash;
    use std::ops::Deref;

    use serde::{ser::SerializeMap, Serialize, Serializer};

    #[derive(Clone, Debug)]
    pub struct OMap<K, V> {
        map: HashMap<K, V>,
        order: Vec<K>
    }

    impl<'a, K, V> OMap<K, V>
        where K: Eq + PartialEq + Hash + Clone
    {
        pub fn new() -> Self {
            OMap {
                map: HashMap::new(),
                order: Vec::new()
            }
        }

        pub fn with_capacity(capacity: usize) -> Self {
            OMap {
                map: HashMap::with_capacity(capacity),
                order: Vec::with_capacity(capacity)
            }
        }

        #[inline]
        pub fn insert(&mut self, key: K, value: V) {
            if self.order.contains(&key) {
                let index = self.order
                    .iter()
                    .position(|x| *x == key)
                    .unwrap();
                self.order.remove(index);
            }

            self.order.push(key.clone());
            self.map.insert(key, value);
        }

        #[inline]
        pub fn to_serialize(&'a self) -> Vec<(&'a K, &'a V)> {
            self.order
                .iter()
                .map(|v| (v, self.map.get(v).unwrap()))
                .collect::<Vec<(&'a K, &'a V)>>()
        }
    }

    impl<'a, K, V> Deref for OMap<K, V> {
        type Target = HashMap<K, V>;

        fn deref(&self) -> &Self::Target {
            &self.map
        }
    }

    impl<K, V> Serialize for OMap<K, V>
        where
            K: Serialize + Eq + PartialEq + Hash + Clone,
            V: Serialize,
    {

        #[inline]
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let mut map = serializer.serialize_map(Some(self.len()))?;
            for (k, v) in self.to_serialize() {
                map.serialize_entry(k, v)?;
            }
            map.end()
        }
    }
}

pub mod compression {
    use std::{fs::File, path::Path};
    use std::io::{BufReader, BufWriter, Write};

    use zstd::stream::{copy_decode, copy_encode};

    use crate::{Compression, db::DBDump, Decompression, Res};

    impl<'a, T> Compression<T> for DBDump<'a>
        where T: AsRef<Path>
    {
        fn compress(&self, output: &T, level: i32) -> Res<()>{
            let input = serde_json::to_string_pretty(&*self)?;
            compress(input.as_bytes(), output, level)?;
            Ok(())
        }
    }

    impl<'a, T> Decompression<T> for DBDump<'a>
        where T: AsRef<Path>
    {
        fn decompress(_input: &T) -> Res<Box<Self>> {
            unimplemented!()
        }
    }

    const BUF_SIZE: usize = 1024 * 1024 * 8;

    fn compress<T>(input: &[u8], output_file: &T, level: i32) -> Res<()>
        where T: AsRef<Path>
    {
        let output_file = File::create(output_file)?;

        let mut reader = BufReader::with_capacity(BUF_SIZE, input);
        let mut writer = BufWriter::new(output_file);

        copy_encode(&mut reader, &mut writer, level)?;

        writer.flush()?;

        Ok(())
    }

    fn _decompress<I, O>(input_file: &I, output_file: &O) -> Res<()>
        where
            I: AsRef<Path>,
            O: AsRef<Path>
    {
        let input_file = File::open(input_file)?;
        let output_file = File::create(output_file)?;

        let mut reader = BufReader::with_capacity(BUF_SIZE, input_file);
        let mut writer = BufWriter::new(output_file);

        copy_decode(&mut reader, &mut writer)?;

        writer.flush()?;

        Ok(())
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

        pub fn build_file<T>(name: T, parents: Option<Vec<String>>) -> File
            where T: ToString
        {
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

        /// Get id of the newest file in the folder.
        /// Required to restore.
        pub async fn get_last_file_id(&self, folder_id: &str) -> Res<(String, String)> {
            log::info!("Getting id of the last file, folder id: '{}'", folder_id);
            let start = time::Instant::now();

            let q = format!("'{}' in parents", folder_id);
            let (resp, files) = self.hub
                .files()
                .list()
                .param("fields", "files(id,name,createdTime)")
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
                Some(files) if files.len() > 0 => files,
                _ => {
                    let msg = "Could not get files, response body is empty".to_string();
                    return Err(Errors::StorageError(msg));
                }
            };
            log::debug!("{} files found, sort and get the last one", files.len());

            files.sort_by(
                |prev, next| {
                    // created_time must exist
                    next.created_time.unwrap().cmp(&prev.created_time.unwrap())
                }
            );
            // if list is not empty we can get the first elem
            let mut first = files.into_iter().nth(0).unwrap();
            // field id, name must exist
            let file_id = first.id.take().unwrap();
            let file_name = first.name.take().unwrap();
            log::info!("File name '{}', id '{}' got for {:?}", file_name, file_id, start.elapsed());

            Ok((file_id, file_name))
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
                let file_name = path.file_name().unwrap().to_str().unwrap();
                GoogleDrive::build_file(file_name, Some(vec![folder_id]))
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

        async fn download(&self, folder_name: &str, prefix: &str) -> Res<String> {
            let folder_id = self.get_file_id(folder_name).await?;
            let (file_id, file_name) = self.get_last_file_id(&folder_id).await?;

            let path = format!("{}{}", prefix, file_name);
            self.download_file(&file_id, Path::new(&path)).await?;

            Ok(path)
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

    impl From<sqlx::Error> for Errors {
        fn from(value: sqlx::Error) -> Self {
            Self::DumpError(value.to_string())
        }
    }

    impl From<tokio::task::JoinError> for Errors {
        fn from(value: tokio::task::JoinError) -> Self {
            Self::DumpError(value.to_string())
        }
    }

    impl From<serde_json::Error> for Errors {
        fn from(value: serde_json::Error) -> Self {
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

#[cfg(test)]
mod test_logger {
    use crate::logger;

    #[test]
    fn test_init() {
        logger::init();
    }
}

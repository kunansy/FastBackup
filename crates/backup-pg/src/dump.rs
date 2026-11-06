use std::time;
use std::{collections::HashMap, fmt::Display, sync::Arc};

use chrono::{NaiveDate, NaiveDateTime};
use serde_json::Value;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgRow, PgSslMode, PgTypeKind};
use sqlx::types::Uuid;
use sqlx::{Column, PgPool, Pool, Postgres, Row, TypeInfo, query};
use tokio::join;

use backup_core::{DbConfig, Errors, Res, compression::compress};

use crate::ordered_map::OMap;

type RowDump = HashMap<String, Value>;
type TableDump = Vec<RowDump>;
pub type DBDump = OMap<String, TableDump>;

#[derive(sqlx::Type, Debug)]
#[sqlx(type_name = "materialtypesenum", rename_all = "lowercase")]
enum MaterialTypesEnum {
    Book,
    Article,
    Course,
    Lecture,
    Audiobook,
}

impl std::fmt::Display for MaterialTypesEnum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let v = match self {
            MaterialTypesEnum::Book => "book",
            MaterialTypesEnum::Article => "article",
            MaterialTypesEnum::Course => "course",
            MaterialTypesEnum::Lecture => "lecture",
            MaterialTypesEnum::Audiobook => "audiobook",
        };
        f.write_str(v)
    }
}

pub async fn prepare_dump<T>(cfg: &T, comp_level: i32) -> Res<(Vec<u8>, String)>
where
    T: DbConfig,
{
    log::info!("Prepare db, dump it");

    let arc_pool = {
        let pool = init_pool(cfg).await?;
        Arc::new(pool)
    };

    let dump = dump(arc_pool.clone(), comp_level).await?;

    let filename = {
        let db_name = arc_pool
            .connect_options()
            .get_database()
            .unwrap_or("undefined")
            .to_string();

        create_filename(&db_name, &None)
    };

    log::info!("DB dumped");

    Ok((dump, filename))
}

pub async fn healthcheck<T>(cfg: &T) -> bool
where
    T: DbConfig,
{
    let pool = match init_pool(cfg).await {
        Ok(pool) => pool,
        Err(e) => {
            log::warn!("Could not init pool: {:?}", e);
            return false;
        }
    };

    _healthcheck(&pool).await
}

async fn _healthcheck(pool: &PgPool) -> bool {
    sqlx::query("SELECT 1 + 1 = 2 AS res")
        .fetch_one(pool)
        .await
        .is_ok_and(|e| e.get("res"))
}

pub async fn dump(pool: Arc<PgPool>, compression_level: i32) -> Res<Vec<u8>> {
    log::info!("Start dumping");
    let start = time::Instant::now();

    let (tables, table_refs) = join!(get_tables(&pool), get_table_refs(&pool));
    let (tables, table_refs) = (tables?, table_refs?);
    let tables_order = define_tables_order(tables, table_refs);

    let dump = dump_all(pool.clone(), tables_order).await?;

    log::debug!("Dump completed, compressing");
    let start_comp = time::Instant::now();
    let compressed = compress_dump(&dump, compression_level)?;
    log::debug!("Compression completed for {:?}", start_comp.elapsed());

    log::info!("Dump completed for {:?}", start.elapsed());
    Ok(compressed)
}

pub async fn init_pool<T>(cfg: &T) -> Res<Pool<Postgres>>
where
    T: DbConfig,
{
    let conn = PgConnectOptions::new()
        .host(cfg.db_host())
        .port(cfg.db_port())
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
        .map_err(db_error)
}

async fn get_tables(pool: &PgPool) -> Res<Vec<String>> {
    log::debug!("Getting tables");

    let tables = sqlx::query(
        "SELECT tablename FROM pg_catalog.pg_tables \
        WHERE schemaname != 'pg_catalog' AND schemaname != 'information_schema';",
    )
    .fetch_all(pool)
    .await
    .map_err(db_error)?
    .into_iter()
    .map(|r| r.get("tablename"))
    .collect::<Vec<String>>();

    log::debug!("{} tables got", tables.len());
    Ok(tables)
}

/// Get pairs (table, ref_to)
async fn get_table_refs(pool: &PgPool) -> Res<HashMap<String, String>> {
    log::debug!("Getting table refs");

    let refs = sqlx::query(
        "SELECT
            --- this is a table
            tc.table_name,

            --- this is a table TO which the table refers
            ccu.table_name AS foreign_table_name
        FROM
            information_schema.table_constraints tc
        JOIN information_schema.constraint_column_usage ccu
            ON ccu.constraint_name = tc.constraint_name
            AND ccu.table_schema = tc.table_schema
        WHERE 
            tc.table_schema != 'pg_catalog' and tc.table_name != ccu.table_name;",
    )
    .fetch_all(pool)
    .await
    .map_err(db_error)?
    .into_iter()
    .map(|r| (r.get("table_name"), r.get("foreign_table_name")))
    // hash map might be used because in the query
    // (table, ref TO) every row represents a single reference
    .collect::<HashMap<String, String>>();

    log::debug!("{} table refs got", refs.len());

    Ok(refs)
}

fn define_tables_order(tables: Vec<String>, table_refs: HashMap<String, String>) -> Vec<String> {
    // TODO: create dependency graph to define the order

    let mut weights = HashMap::new();
    // how many links to the table
    for table_name in table_refs.into_values() {
        *weights.entry(table_name).or_insert(0) += 1;
    }
    // there might be tables without links, add them
    for table_name in tables {
        weights.entry(table_name).or_insert(0);
    }

    let mut order = weights.into_iter().collect::<Vec<(String, i32)>>();

    // descending order by refs count
    order.sort_by(|(_, v), (_, v2)| v2.cmp(v));

    order.into_iter().map(|(k, _)| k).collect::<Vec<String>>()
}

async fn dump_all(pool: Arc<PgPool>, tables: Vec<String>) -> Res<DBDump> {
    // save order of the tables with OMap
    let mut table_dumps = OMap::with_capacity(tables.len());

    let tasks = tables.into_iter().map(|table| {
        // clone is not escapable here
        let cp = table.clone();
        (table, tokio::spawn(dump_table(pool.clone(), cp)))
    });

    // run all tasks concurrently
    for (table, task) in tasks.into_iter() {
        let table_dump = task.await.map_err(|e| Errors::DumpError(e.to_string()))??;
        table_dumps.insert(table, table_dump);
    }

    Ok(table_dumps)
}

#[inline]
async fn dump_table<T>(pool: Arc<PgPool>, table_name: T) -> Res<TableDump>
where
    T: Display,
{
    let res = query(sqlx::AssertSqlSafe(format!("SELECT * FROM {table_name}")))
        .fetch_all(&*pool)
        .await
        .map_err(db_error)?
        .into_iter()
        .map(dump_row)
        .collect::<Vec<RowDump>>();

    Ok(res)
}

#[inline]
fn dump_row(row: PgRow) -> RowDump {
    let mut res = HashMap::with_capacity(row.columns().len());

    for column in row.columns() {
        let name = column.name();
        let value = match column.type_info().name() {
            "INT4" => match row.get::<Option<i32>, _>(name) {
                Some(v) => Value::Number(v.into()),
                None => Value::Null,
            },
            "INT8" => match row.get::<Option<i64>, _>(name) {
                Some(v) => Value::Number(v.into()),
                None => Value::Null,
            },
            "VARCHAR" | "TEXT" => match row.get::<Option<String>, _>(name) {
                Some(v) => Value::String(v),
                None => Value::Null,
            },
            "UUID" => match row.get::<Option<Uuid>, _>(name) {
                Some(v) => Value::String(v.to_string()),
                None => Value::Null,
            },
            "BOOL" => match row.get::<Option<bool>, _>(name) {
                Some(v) => Value::Bool(v),
                None => Value::Null,
            },
            "TIMESTAMP" => match row.get::<Option<NaiveDateTime>, _>(name) {
                Some(v) => Value::String(v.format("%Y-%m-%d %H:%M:%S").to_string()),
                None => Value::Null,
            },
            "JSONB" | "JSON" => row.get::<Value, _>(name),
            "DATE" => match row.get::<Option<NaiveDate>, _>(name) {
                Some(v) => Value::String(v.format("%Y-%m-%d").to_string()),
                None => Value::Null,
            },
            // TODO: how to match type without name??
            "materialtypesenum" => match row.get::<Option<MaterialTypesEnum>, _>(name) {
                Some(v) => Value::String(v.to_string()),
                None => Value::Null,
            },
            _ => match column.type_info().kind() {
                PgTypeKind::Enum(_) => panic!("Enum column: {:?}, could not process", column),
                v => panic!(
                    "Not processed type: {:?}, {:?}",
                    v,
                    column.type_info().name()
                ),
            },
        };

        res.insert(name.to_string(), value);
    }
    res
}

fn db_error(e: sqlx::Error) -> Errors {
    Errors::DumpError(e.to_string())
}

fn compress_dump(dump: &DBDump, level: i32) -> Res<Vec<u8>> {
    let input = serde_json::to_string_pretty(dump).map_err(|e| Errors::DumpError(e.to_string()))?;

    compress(input.as_bytes(), level)
}

fn create_filename(db_name: &str, folder: &Option<String>) -> String {
    let prefix = match folder {
        Some(v) => format!("{}/", v),
        None => "".to_string(),
    };
    format!(
        "{}backup_{}_{}.dump",
        prefix,
        db_name,
        chrono::Utc::now().format("%Y-%m-%d_%H:%M:%S")
    )
}

#[cfg(test)]
mod test_db {

    use crate::dump as db;

    #[test]
    fn test_create_filename_with_folder() {
        let f = db::create_filename("tdb", &Some("tf".to_string()));

        assert!(f.starts_with("tf/backup_tdb_"));
        assert!(f.ends_with(".dump"));
    }

    #[test]
    fn test_create_filename_without_folder() {
        let f = db::create_filename("tdb", &None);

        assert!(f.starts_with("backup_tdb_"));
        assert!(f.ends_with(".dump"));
    }
}

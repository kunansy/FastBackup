pub mod db {
    use std::collections::HashMap;
    use std::time;
    use sqlx::{PgPool, postgres::PgPoolOptions};

    type SqlxRes<T> = Result<T, sqlx::Error>;

    pub async fn init_pool(uri: &str, timeout: time::Duration) -> SqlxRes<PgPool> {
        PgPoolOptions::new()
            .max_connections(5)
            .idle_timeout(timeout)
            .acquire_timeout(timeout)
            .connect(uri).await
    }

    pub async fn get_tables(pool: &PgPool) -> SqlxRes<Vec<String>> {
        log::info!("Getting tables");

        let tables = sqlx::query!(
            "SELECT tablename FROM pg_catalog.pg_tables \
            WHERE schemaname != 'pg_catalog' AND schemaname != 'information_schema';")
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|r| r.tablename.unwrap())
            .collect::<Vec<String>>();

        log::info!("{} tables got", tables.len());
        Ok(tables)
    }

    pub async fn get_table_refs(pool: &PgPool) -> SqlxRes<HashMap<String, String>> {
        log::info!("Getting table refs");

        let refs = sqlx::query!(
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
            .map(|r| (r.table_name.unwrap(), r.foreign_table_name.unwrap()))
            .collect::<HashMap<String, String>>();

        log::info!("{} table refs got", refs.len());

        Ok(refs)
    }

    pub fn define_tables_order<'a>(tables: &'a Vec<String>,
                                   table_refs: &'a HashMap<String, String>) -> Vec<&'a String> {
        let mut weights = HashMap::new();
        for table_name in table_refs.values() {
            *weights.entry(table_name).or_insert(0) += 1;
        }
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
}
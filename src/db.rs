pub mod db {
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
            WHERE schemaname != 'pg_catalog' AND schemaname != 'information_schema'; ")
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|r| r.tablename.unwrap())
            .collect::<Vec<String>>();

        log::info!("{} tables got", tables.len());
        Ok(tables)
    }
}
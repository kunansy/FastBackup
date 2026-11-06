//! Needs PostgreSQL 18 and its client binaries in PATH.
//! Run: `PG_TEST_HOST=... cargo test -p backup-pg -- --ignored`.
//! The user must be able to create databases.

use std::sync::Arc;

use backup_core::{DbHealth, DbTarget, Dumper, Restorer, Secret};
use backup_pg::{PgTools, TokioCommandRunner, ZstdLevel};

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn tools_for(db_name: &str) -> PgTools {
    let target = DbTarget {
        host: env_or("PG_TEST_HOST", "localhost"),
        port: env_or("PG_TEST_PORT", "5432")
            .parse()
            .expect("PG_TEST_PORT"),
        user: env_or("PG_TEST_USER", "postgres"),
        password: Secret::new(env_or("PG_TEST_PASSWORD", "postgres")),
        name: db_name.to_string(),
    };
    PgTools::new(target, ZstdLevel::default(), Arc::new(TokioCommandRunner))
}

/// Tests run in parallel, so each one gets its own database.
async fn fresh_db(name: &str) -> PgTools {
    let admin = tools_for(&env_or("PG_TEST_ADMIN_DB", "postgres"));
    admin
        .query(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"))
        .await
        .expect("old test database dropped");
    admin
        .query(&format!("CREATE DATABASE {name}"))
        .await
        .expect("test database created");
    tools_for(name)
}

const SCHEMA: &str = "
CREATE TYPE materialtypesenum AS ENUM ('book', 'article', 'audiobook');
CREATE TABLE materials (
    material_id serial PRIMARY KEY,
    title text NOT NULL,
    material_type materialtypesenum NOT NULL,
    tags jsonb,
    added_at timestamp NOT NULL DEFAULT now()
);
INSERT INTO materials (title, material_type, tags) VALUES
    ('Dune', 'book', '[\"sf\"]'),
    ('Rust blog', 'article', NULL),
    ('Hobbit', 'audiobook', '{\"narrator\": \"x\"}');
CREATE MATERIALIZED VIEW mvw_types AS
    SELECT material_type, count(*) AS cnt FROM materials GROUP BY material_type;
";

const CHANGES_AFTER_BACKUP: &str = "
INSERT INTO materials (title, material_type) VALUES ('Lost', 'book');
DROP MATERIALIZED VIEW mvw_types;
ALTER TYPE materialtypesenum ADD VALUE 'course';
";

#[tokio::test]
#[ignore = "requires PostgreSQL 18 server and client binaries"]
async fn test_dump_and_restore_roundtrip() {
    let pg = fresh_db("backup_test_roundtrip").await;
    pg.ping().await.expect("database is reachable");
    pg.query(SCHEMA).await.expect("schema created");

    let dir = tempfile::tempdir().unwrap();
    let dump = dir.path().join("backup.pgdump");
    pg.dump(&dump).await.expect("dump succeeds");
    assert!(std::fs::metadata(&dump).unwrap().len() > 0);

    pg.query(CHANGES_AFTER_BACKUP)
        .await
        .expect("changes applied");
    pg.restore(&dump).await.expect("restore succeeds");

    let titles = pg
        .query("SELECT string_agg(title, ',' ORDER BY material_id) FROM materials")
        .await
        .unwrap();
    assert_eq!(titles.trim(), "Dune,Rust blog,Hobbit");

    let next_id = pg
        .query("SELECT nextval('materials_material_id_seq')")
        .await
        .unwrap();
    assert_eq!(next_id.trim(), "4");

    let view = pg
        .query("SELECT string_agg(material_type || '=' || cnt, ',' ORDER BY material_type) FROM mvw_types")
        .await
        .unwrap();
    assert_eq!(view.trim(), "book=1,article=1,audiobook=1");

    let enum_values = pg
        .query("SELECT string_agg(enumlabel, ',' ORDER BY enumsortorder) FROM pg_enum WHERE enumtypid = 'materialtypesenum'::regtype")
        .await
        .unwrap();
    assert_eq!(enum_values.trim(), "book,article,audiobook");

    let jsonb = pg
        .query("SELECT tags->>'narrator' FROM materials WHERE title = 'Hobbit'")
        .await
        .unwrap();
    assert_eq!(jsonb.trim(), "x");
}

#[tokio::test]
#[ignore = "requires PostgreSQL 18 server and client binaries"]
async fn test_restore_of_broken_file_keeps_database_intact() {
    let pg = fresh_db("backup_test_broken").await;
    pg.query(SCHEMA).await.expect("schema created");

    let dir = tempfile::tempdir().unwrap();
    let broken = dir.path().join("broken.pgdump");
    std::fs::write(&broken, b"PGDMP but not really").unwrap();

    assert!(pg.restore(&broken).await.is_err());

    let count = pg.query("SELECT count(*) FROM materials").await.unwrap();
    assert_eq!(count.trim(), "3");
}

#[tokio::test]
#[ignore = "requires PostgreSQL 18 server and client binaries"]
async fn test_restore_into_empty_database() {
    let source = fresh_db("backup_test_source").await;
    source.query(SCHEMA).await.expect("schema created");
    let dir = tempfile::tempdir().unwrap();
    let dump = dir.path().join("backup.pgdump");
    source.dump(&dump).await.expect("dump succeeds");

    let empty = fresh_db("backup_test_empty").await;

    empty.restore(&dump).await.expect("restore succeeds");

    let count = empty.query("SELECT count(*) FROM mvw_types").await.unwrap();
    assert_eq!(count.trim(), "3");
}

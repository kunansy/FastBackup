# FastBackup
![Tests status](https://github.com/kunansy/fastbackup/actions/workflows/linters.yml/badge.svg)
![Build Status](https://github.com/kunansy/fastbackup/actions/workflows/buildx-docker-image.yml/badge.svg)

gRPC service that backs up one PostgreSQL database to a Google Drive folder
and restores it.

Backups are made with `pg_dump --format=custom --compress=zstd` and restored with
`pg_restore --clean --if-exists --single-transaction`, so schema, data, enums,
sequences and materialized views come back as they were. Before every restore
the current database is uploaded as a `pre_restore_*` copy.

Backups written by FastBackup 0.5 (zstd-compressed JSON, `*.dump`) are still
listed and downloadable; `DownloadBackup` streams them decompressed. They cannot
be restored by the server.

## API

See [`crates/backup-proto/proto/backup.proto`](crates/backup-proto/proto/backup.proto).

| RPC | Purpose |
|-----|---------|
| `CreateBackup` | Dump the database and upload it; apply retention. |
| `ListBackups` | Newest first, paginated. |
| `GetBackup` | Metadata of a backup by id or the latest one. |
| `DownloadBackup` | Stream a backup by id or the latest one. |
| `Restore` | Overwrite the database; requires `confirm_db_name`. |
| `Healthcheck` | Whether the database accepts connections. |

Backup and restore are serialized: a concurrent call gets `ABORTED`.

```sh
grpcurl -plaintext -import-path crates/backup-proto/proto -proto backup.proto \
  -d '{"backup": {"latest": true}, "confirm_db_name": "tracker"}' \
  localhost:50051 backup.Backup/Restore
```

## Configuration

Environment variables, see [`env.example`](env.example).
The image ships `postgresql-client-18`; its major version must not be lower than the server's.

## Layout

| Crate | Contents |
|-------|----------|
| `backup-core` | Domain types, ports (`Dumper`, `Restorer`, `DbHealth`, `Storage`), errors. |
| `backup-pg` | `pg_dump` / `pg_restore` / `psql` runner. |
| `backup-gdrive` | Google Drive `Storage`. |
| `backup-proto` | gRPC definitions. |
| `backup-server` | Configuration, use cases, gRPC server. |

## Development

Dependencies are vendored; after changing them run `cargo vendor` with
`.cargo/config.toml` temporarily moved away, then restore it.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# Integration tests: PostgreSQL 18 server, its client binaries in PATH.
PG_TEST_HOST=localhost cargo test --workspace -- --include-ignored --skip drive_smoke
# Also round-trip a real database (read-only for it; copies go to backup_test_* databases).
PG_TEST_SOURCE_DB=tracker PG_TEST_HOST=localhost cargo test -p backup-pg --test roundtrip -- --ignored
# Real Google Drive.
DRIVE_TEST_CREDS="$(cat key.json)" DRIVE_TEST_FOLDER_ID=... cargo test -p backup-gdrive -- --ignored
```

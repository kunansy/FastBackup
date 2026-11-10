use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use backup_core::{
    BackupFormat, BackupInfo, BackupKind, Clock, DbHealth, Dumper, Error, Page, Restorer, Result,
    Storage, UploadRequest, compression::decompress_file, retention_candidates,
};
use tempfile::NamedTempFile;
use tokio::sync::{Mutex, MutexGuard};

const DEFAULT_PAGE_SIZE: u32 = 20;
const MAX_PAGE_SIZE: u32 = 100;
/// Upper bound for the retention scan, so a misbehaving storage cannot loop forever.
const MAX_RETENTION_PAGES: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackupTarget {
    Id(String),
    Latest,
}

pub struct ServiceDeps {
    pub dumper: Arc<dyn Dumper>,
    pub restorer: Arc<dyn Restorer>,
    pub health: Arc<dyn DbHealth>,
    pub storage: Arc<dyn Storage>,
    pub clock: Arc<dyn Clock>,
    pub data_folder: PathBuf,
    pub retention: Option<usize>,
}

#[derive(Debug)]
pub struct RestoreOutcome {
    pub restored: BackupInfo,
    pub pre_restore: BackupInfo,
}

/// Backup use cases for one database and one storage.
pub struct BackupService {
    deps: ServiceDeps,
    /// Serializes operations that dump or overwrite the database.
    write_lock: Mutex<()>,
}

impl BackupService {
    pub fn new(deps: ServiceDeps) -> Self {
        Self {
            deps,
            write_lock: Mutex::new(()),
        }
    }

    pub fn db_name(&self) -> &str {
        self.deps.restorer.db_name()
    }

    fn lock(&self) -> Result<MutexGuard<'_, ()>> {
        self.write_lock
            .try_lock()
            .map_err(|_| Error::Busy("another backup or restore is in progress".into()))
    }

    fn temp_file(&self, suffix: &str) -> Result<NamedTempFile> {
        tempfile::Builder::new()
            .prefix("backup_")
            .suffix(suffix)
            .tempfile_in(&self.deps.data_folder)
            .map_err(|e| Error::io("Could not create temporary file", e))
    }

    pub async fn create_backup(&self) -> Result<BackupInfo> {
        let _guard = self.lock()?;
        let info = self.dump_and_upload(BackupKind::Regular).await?;
        self.prune_old_backups().await;
        Ok(info)
    }

    async fn dump_and_upload(&self, kind: BackupKind) -> Result<BackupInfo> {
        let file = self.temp_file(".pgdump")?;
        // The TLS handshake and token refresh overlap the dump instead of delaying the upload.
        let (dumped, warmed) = tokio::join!(
            self.deps.dumper.dump(file.path()),
            self.deps.storage.warm_up()
        );
        dumped?;
        if let Err(e) = warmed {
            log::warn!("Could not warm up storage: {e}");
        }

        let req = UploadRequest::pg_dump(
            file.path(),
            self.deps.dumper.db_name(),
            kind,
            self.deps.clock.now(),
        );
        self.deps.storage.upload(req).await
    }

    /// Best effort: the new backup is already stored, so failures are only logged.
    async fn prune_old_backups(&self) {
        let Some(keep) = self.deps.retention else {
            return;
        };
        let all = match self.list_all().await {
            Ok(all) => all,
            Err(e) => {
                log::error!("Retention skipped, could not list backups: {e}");
                return;
            }
        };
        for backup in retention_candidates(&all, keep) {
            match self.deps.storage.delete(&backup.id).await {
                Ok(()) => log::info!("Retention removed '{}'", backup.name),
                Err(e) => log::error!("Retention could not remove '{}': {e}", backup.name),
            }
        }
    }

    async fn list_all(&self) -> Result<Vec<BackupInfo>> {
        let mut all = Vec::new();
        let mut token: Option<String> = None;
        for _ in 0..MAX_RETENTION_PAGES {
            let page = self
                .deps
                .storage
                .list(MAX_PAGE_SIZE, token.as_deref())
                .await?;
            all.extend(page.items);
            match page.next_page_token {
                Some(next) if !next.is_empty() => token = Some(next),
                _ => return Ok(all),
            }
        }
        Err(Error::Storage(format!(
            "more than {MAX_RETENTION_PAGES} pages of backups"
        )))
    }

    pub async fn list(&self, page_size: u32, page_token: Option<&str>) -> Result<Page<BackupInfo>> {
        let page_size = match page_size {
            0 => DEFAULT_PAGE_SIZE,
            n => n.min(MAX_PAGE_SIZE),
        };
        self.deps.storage.list(page_size, page_token).await
    }

    pub async fn resolve(&self, target: &BackupTarget) -> Result<BackupInfo> {
        match target {
            BackupTarget::Id(id) => self.deps.storage.get(id).await,
            BackupTarget::Latest => self
                .deps
                .storage
                .latest()
                .await?
                .ok_or_else(|| Error::NotFound("no backups".into())),
        }
    }

    /// The returned file is removed on drop. Legacy backups come decompressed.
    pub async fn download(&self, target: &BackupTarget) -> Result<(BackupInfo, NamedTempFile)> {
        let info = self.resolve(target).await?;
        let file = self.temp_file(".download")?;
        self.deps.storage.download(&info, file.path()).await?;

        if info.format != BackupFormat::LegacyJsonZstd {
            return Ok((info, file));
        }
        let json = self.temp_file(".json")?;
        let (src, dst) = (file.path().to_owned(), json.path().to_owned());
        tokio::task::spawn_blocking(move || decompress_file(&src, &dst))
            .await
            .map_err(|e| Error::Storage(format!("Decompression task failed: {e}")))??;
        Ok((info, json))
    }

    pub async fn restore(
        &self,
        target: &BackupTarget,
        confirm_db_name: &str,
    ) -> Result<RestoreOutcome> {
        if confirm_db_name != self.db_name() {
            return Err(Error::FailedPrecondition(format!(
                "confirm_db_name '{confirm_db_name}' does not match the configured database"
            )));
        }
        let _guard = self.lock()?;
        let start = Instant::now();

        let restored = self.resolve(target).await?;
        if restored.format != BackupFormat::PgCustom {
            return Err(Error::FailedPrecondition(format!(
                "'{}' is {}; restore it with DownloadBackup and the legacy client",
                restored.name,
                restored.format.as_str()
            )));
        }
        let file = self.temp_file(".pgdump")?;
        self.deps.storage.download(&restored, file.path()).await?;

        let pre_restore = self.dump_and_upload(BackupKind::PreRestore).await?;
        log::info!("Pre-restore copy '{}' stored", pre_restore.name);

        self.deps.restorer.restore(file.path()).await?;
        log::info!(
            "Database '{}' restored from '{}' for {:?}",
            self.db_name(),
            restored.name,
            start.elapsed()
        );
        Ok(RestoreOutcome {
            restored,
            pre_restore,
        })
    }

    pub async fn healthcheck(&self) -> bool {
        match self.deps.health.ping().await {
            Ok(()) => true,
            Err(e) => {
                log::error!("Database healthcheck failed: {e}");
                false
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use backup_core::{
        BackupFormat, BackupInfo, BackupKind, Clock, DbHealth, Dumper, Error, Page, Restorer,
        Result, Storage, UploadRequest,
    };
    use chrono::{DateTime, Duration, TimeZone, Utc};
    use tokio::sync::Notify;

    use super::{BackupService, BackupTarget, ServiceDeps};

    pub(crate) struct FixedClock(pub DateTime<Utc>);

    impl Clock for FixedClock {
        fn now(&self) -> DateTime<Utc> {
            self.0
        }
    }

    /// In-memory storage; content lives in a map keyed by id.
    #[derive(Default)]
    pub(crate) struct FakeStorage {
        pub files: Mutex<Vec<(BackupInfo, Vec<u8>)>>,
        pub deleted: Mutex<Vec<String>>,
        pub delete_calls: AtomicUsize,
        pub events: Arc<Mutex<Vec<String>>>,
        pub warm_up_calls: AtomicUsize,
        pub fail_warm_up: AtomicBool,
        pub fail_upload: AtomicBool,
        pub fail_download: AtomicBool,
        pub fail_list: AtomicBool,
        pub fail_delete: AtomicBool,
    }

    impl FakeStorage {
        pub fn put(&self, info: BackupInfo, content: &[u8]) {
            self.files.lock().unwrap().push((info, content.to_vec()));
        }

        fn infos(&self) -> Vec<BackupInfo> {
            let mut infos: Vec<BackupInfo> = self
                .files
                .lock()
                .unwrap()
                .iter()
                .map(|(i, _)| i.clone())
                .collect();
            infos.sort_by_key(|i| std::cmp::Reverse(i.created_at));
            infos
        }

        fn check(flag: &AtomicBool, op: &str) -> Result<()> {
            match flag.load(Ordering::SeqCst) {
                true => Err(Error::Unavailable(format!("{op} failed"))),
                false => Ok(()),
            }
        }
    }

    #[async_trait::async_trait]
    impl Storage for FakeStorage {
        async fn warm_up(&self) -> Result<()> {
            self.warm_up_calls.fetch_add(1, Ordering::SeqCst);
            Self::check(&self.fail_warm_up, "warm up")
        }

        async fn upload(&self, req: UploadRequest<'_>) -> Result<BackupInfo> {
            Self::check(&self.fail_upload, "upload")?;
            let content = std::fs::read(req.path()).unwrap();
            let n = self.files.lock().unwrap().len();
            let info = BackupInfo {
                id: format!("up{n}"),
                name: req.name().to_string(),
                created_at: Utc.with_ymd_and_hms(2030, 1, 1, 0, 0, 0).unwrap()
                    + Duration::seconds(n as i64),
                size_bytes: Some(content.len() as u64),
                md5: None,
                format: req.format(),
                kind: req.kind(),
            };
            self.events
                .lock()
                .unwrap()
                .push(format!("upload {}", req.kind().as_str()));
            self.put(info.clone(), &content);
            Ok(info)
        }

        async fn list(&self, page_size: u32, page_token: Option<&str>) -> Result<Page<BackupInfo>> {
            Self::check(&self.fail_list, "list")?;
            let skip: usize = page_token.map_or(0, |t| t.parse().unwrap());
            let infos = self.infos();
            let end = (skip + page_size as usize).min(infos.len());
            Ok(Page {
                items: infos[skip..end].to_vec(),
                next_page_token: (end < infos.len()).then(|| end.to_string()),
            })
        }

        async fn latest(&self) -> Result<Option<BackupInfo>> {
            Ok(self
                .infos()
                .into_iter()
                .find(|i| i.kind != BackupKind::PreRestore))
        }

        async fn get(&self, id: &str) -> Result<BackupInfo> {
            self.infos()
                .into_iter()
                .find(|i| i.id == id)
                .ok_or_else(|| Error::NotFound(id.into()))
        }

        async fn download(&self, backup: &BackupInfo, dst: &Path) -> Result<()> {
            Self::check(&self.fail_download, "download")?;
            let files = self.files.lock().unwrap();
            let (_, content) = files.iter().find(|(i, _)| i.id == backup.id).unwrap();
            std::fs::write(dst, content).unwrap();
            Ok(())
        }

        async fn delete(&self, id: &str) -> Result<()> {
            self.delete_calls.fetch_add(1, Ordering::SeqCst);
            Self::check(&self.fail_delete, "delete")?;
            self.files.lock().unwrap().retain(|(i, _)| i.id != id);
            self.deleted.lock().unwrap().push(id.to_string());
            Ok(())
        }
    }

    /// Database fake: dumps `content`, records restores, can block a dump.
    pub(crate) struct FakeDb {
        pub content: Vec<u8>,
        pub restored: Mutex<Vec<Vec<u8>>>,
        pub healthy: bool,
        pub fail_dump: bool,
        pub fail_restore: bool,
        pub dump_gate: Option<Arc<Notify>>,
        pub events: Arc<Mutex<Vec<String>>>,
    }

    impl FakeDb {
        pub fn new(events: Arc<Mutex<Vec<String>>>) -> Self {
            Self {
                content: b"current-db".to_vec(),
                restored: Mutex::new(Vec::new()),
                healthy: true,
                fail_dump: false,
                fail_restore: false,
                dump_gate: None,
                events,
            }
        }
    }

    fn process_error(program: &str) -> Error {
        Error::Process {
            program: program.into(),
            status: "exit status: 1".into(),
            stderr: "boom".into(),
        }
    }

    #[async_trait::async_trait]
    impl Dumper for FakeDb {
        fn db_name(&self) -> &str {
            "tracker"
        }

        async fn dump(&self, dst: &Path) -> Result<()> {
            if let Some(gate) = &self.dump_gate {
                gate.notified().await;
            }
            if self.fail_dump {
                return Err(process_error("pg_dump"));
            }
            self.events.lock().unwrap().push("dump".into());
            std::fs::write(dst, &self.content).unwrap();
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl Restorer for FakeDb {
        fn db_name(&self) -> &str {
            "tracker"
        }

        async fn restore(&self, src: &Path) -> Result<()> {
            self.events.lock().unwrap().push("restore".into());
            if self.fail_restore {
                return Err(process_error("pg_restore"));
            }
            self.restored
                .lock()
                .unwrap()
                .push(std::fs::read(src).unwrap());
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl DbHealth for FakeDb {
        async fn ping(&self) -> Result<()> {
            match self.healthy {
                true => Ok(()),
                false => Err(Error::FailedPrecondition("down".into())),
            }
        }
    }

    pub(crate) struct Harness {
        pub service: Arc<BackupService>,
        pub storage: Arc<FakeStorage>,
        pub db: Arc<FakeDb>,
        pub events: Arc<Mutex<Vec<String>>>,
        _dir: tempfile::TempDir,
    }

    pub(crate) fn harness_with(retention: Option<usize>, db: impl FnOnce(&mut FakeDb)) -> Harness {
        let events = Arc::new(Mutex::new(Vec::new()));
        let storage = Arc::new(FakeStorage {
            events: events.clone(),
            ..FakeStorage::default()
        });
        let mut fake_db = FakeDb::new(events.clone());
        db(&mut fake_db);
        let db = Arc::new(fake_db);
        let dir = tempfile::tempdir().unwrap();

        let service = Arc::new(BackupService::new(ServiceDeps {
            dumper: db.clone(),
            restorer: db.clone(),
            health: db.clone(),
            storage: storage.clone(),
            clock: Arc::new(FixedClock(
                Utc.with_ymd_and_hms(2026, 11, 6, 12, 0, 0).unwrap(),
            )),
            data_folder: dir.path().to_owned(),
            retention,
        }));
        Harness {
            service,
            storage,
            db,
            events,
            _dir: dir,
        }
    }

    pub(crate) fn harness() -> Harness {
        harness_with(None, |_| {})
    }

    pub(crate) fn info(id: &str, day: u32, format: BackupFormat, kind: BackupKind) -> BackupInfo {
        BackupInfo {
            id: id.into(),
            name: format!("{id}.file"),
            created_at: Utc.with_ymd_and_hms(2026, 1, day, 0, 0, 0).unwrap(),
            size_bytes: None,
            md5: None,
            format,
            kind,
        }
    }

    fn temp_files_left(h: &Harness) -> usize {
        std::fs::read_dir(h._dir.path()).unwrap().count()
    }

    fn events(h: &Harness) -> Vec<String> {
        h.events.lock().unwrap().clone()
    }

    fn put_good_backup(h: &Harness) {
        h.storage.put(
            info("good", 1, BackupFormat::PgCustom, BackupKind::Regular),
            b"backup-content",
        );
    }

    /// Yields until a spawned operation holds the write lock.
    async fn wait_until_locked(h: &Harness) {
        tokio::task::yield_now().await;
        while h.service.write_lock.try_lock().is_ok() {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn test_create_backup_uploads_dump() {
        let h = harness();

        let created = h.service.create_backup().await.unwrap();

        assert_eq!(created.name, "backup_tracker_2026-11-06_12:00:00.pgdump");
        assert_eq!(created.kind, BackupKind::Regular);
        assert_eq!(created.format, BackupFormat::PgCustom);
        assert_eq!(h.storage.files.lock().unwrap()[0].1, b"current-db");
        assert_eq!(temp_files_left(&h), 0);
    }

    #[tokio::test]
    async fn test_create_backup_survives_failed_warm_up() {
        let h = harness();
        h.storage.fail_warm_up.store(true, Ordering::SeqCst);

        h.service.create_backup().await.unwrap();

        assert_eq!(h.storage.warm_up_calls.load(Ordering::SeqCst), 1);
        assert_eq!(h.storage.files.lock().unwrap()[0].1, b"current-db");
    }

    #[tokio::test]
    async fn test_create_backup_prunes_only_regular_pg_backups() {
        let h = harness_with(Some(2), |_| {});
        for (id, day) in [("old1", 1), ("old2", 2), ("old3", 3)] {
            h.storage.put(
                info(id, day, BackupFormat::PgCustom, BackupKind::Regular),
                b"x",
            );
        }
        h.storage.put(
            info(
                "legacy",
                1,
                BackupFormat::LegacyJsonZstd,
                BackupKind::Regular,
            ),
            b"x",
        );
        h.storage.put(
            info("safety", 1, BackupFormat::PgCustom, BackupKind::PreRestore),
            b"x",
        );

        h.service.create_backup().await.unwrap();

        let mut deleted = h.storage.deleted.lock().unwrap().clone();
        deleted.sort();
        assert_eq!(deleted, ["old1", "old2"]);
    }

    #[tokio::test]
    async fn test_concurrent_backup_is_rejected() {
        let gate = Arc::new(Notify::new());
        let g = gate.clone();
        let h = harness_with(None, move |db| db.dump_gate = Some(g));

        let first = tokio::spawn({
            let service = h.service.clone();
            async move { service.create_backup().await }
        });
        wait_until_locked(&h).await;

        let second = h.service.create_backup().await;
        assert!(matches!(second, Err(Error::Busy(_))), "{second:?}");

        gate.notify_one();
        first.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn test_list_clamps_page_size() {
        let h = harness();
        for day in 1..=25 {
            h.storage.put(
                info(
                    &format!("b{day}"),
                    day,
                    BackupFormat::PgCustom,
                    BackupKind::Regular,
                ),
                b"x",
            );
        }

        let default_page = h.service.list(0, None).await.unwrap();
        let capped = h.service.list(1000, None).await.unwrap();

        assert_eq!(default_page.items.len(), 20);
        assert_eq!(default_page.items[0].id, "b25");
        assert_eq!(capped.items.len(), 25);
    }

    #[tokio::test]
    async fn test_download_decompresses_legacy_backup() {
        let h = harness();
        let json = br#"{"materials": []}"#;
        h.storage.put(
            info(
                "legacy",
                1,
                BackupFormat::LegacyJsonZstd,
                BackupKind::Regular,
            ),
            &zstd::encode_all(json.as_slice(), 3).unwrap(),
        );

        let (got, file) = h.service.download(&BackupTarget::Latest).await.unwrap();

        assert_eq!(got.id, "legacy");
        assert_eq!(std::fs::read(file.path()).unwrap(), json);
    }

    #[tokio::test]
    async fn test_download_unknown_id() {
        let h = harness();

        let err = h
            .service
            .download(&BackupTarget::Id("nope".into()))
            .await
            .unwrap_err();

        assert!(matches!(err, Error::NotFound(_)));
    }

    #[tokio::test]
    async fn test_restore_takes_pre_restore_copy_then_restores() {
        let h = harness();
        h.storage.put(
            info("good", 1, BackupFormat::PgCustom, BackupKind::Regular),
            b"backup-content",
        );

        let outcome = h
            .service
            .restore(&BackupTarget::Id("good".into()), "tracker")
            .await
            .unwrap();

        assert_eq!(outcome.restored.id, "good");
        assert_eq!(outcome.pre_restore.kind, BackupKind::PreRestore);
        assert!(
            outcome
                .pre_restore
                .name
                .starts_with("pre_restore_backup_tracker_")
        );
        assert_eq!(
            *h.events.lock().unwrap(),
            ["dump", "upload pre_restore", "restore"]
        );
        assert_eq!(h.db.restored.lock().unwrap()[0], b"backup-content");
        assert_eq!(temp_files_left(&h), 0);
    }

    #[tokio::test]
    async fn test_restore_requires_matching_db_name() {
        let h = harness();
        h.storage.put(
            info("good", 1, BackupFormat::PgCustom, BackupKind::Regular),
            b"x",
        );

        let err = h
            .service
            .restore(&BackupTarget::Latest, "other")
            .await
            .unwrap_err();

        assert!(matches!(err, Error::FailedPrecondition(_)));
        assert!(h.events.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_restore_rejects_legacy_backup() {
        let h = harness();
        h.storage.put(
            info(
                "legacy",
                1,
                BackupFormat::LegacyJsonZstd,
                BackupKind::Regular,
            ),
            b"x",
        );

        let err = h
            .service
            .restore(&BackupTarget::Latest, "tracker")
            .await
            .unwrap_err();

        assert!(matches!(err, Error::FailedPrecondition(_)));
        assert!(h.events.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_restore_aborts_when_pre_restore_dump_fails() {
        let h = harness_with(None, |db| db.fail_dump = true);
        put_good_backup(&h);

        let err = h
            .service
            .restore(&BackupTarget::Id("good".into()), "tracker")
            .await
            .unwrap_err();

        assert!(matches!(err, Error::Process { .. }), "{err:?}");
        assert!(
            !events(&h).contains(&"restore".to_string()),
            "{:?}",
            events(&h)
        );
        assert!(h.db.restored.lock().unwrap().is_empty());
        assert_eq!(temp_files_left(&h), 0);
    }

    #[tokio::test]
    async fn test_restore_aborts_when_pre_restore_upload_fails() {
        let h = harness();
        put_good_backup(&h);
        h.storage.fail_upload.store(true, Ordering::SeqCst);

        let err = h
            .service
            .restore(&BackupTarget::Id("good".into()), "tracker")
            .await
            .unwrap_err();

        assert!(matches!(err, Error::Unavailable(_)), "{err:?}");
        assert_eq!(events(&h), ["dump"]);
        assert!(h.db.restored.lock().unwrap().is_empty());
        assert_eq!(temp_files_left(&h), 0);
    }

    #[tokio::test]
    async fn test_restore_aborts_when_download_fails() {
        let h = harness();
        put_good_backup(&h);
        h.storage.fail_download.store(true, Ordering::SeqCst);

        let err = h
            .service
            .restore(&BackupTarget::Id("good".into()), "tracker")
            .await
            .unwrap_err();

        assert!(matches!(err, Error::Unavailable(_)), "{err:?}");
        assert!(events(&h).is_empty(), "{:?}", events(&h));
        assert_eq!(temp_files_left(&h), 0);
    }

    #[tokio::test]
    async fn test_failed_restore_keeps_pre_restore_copy_and_releases_lock() {
        let h = harness_with(None, |db| db.fail_restore = true);
        put_good_backup(&h);

        let err = h
            .service
            .restore(&BackupTarget::Id("good".into()), "tracker")
            .await
            .unwrap_err();

        assert!(matches!(err, Error::Process { .. }), "{err:?}");
        let kinds: Vec<BackupKind> = h.storage.infos().iter().map(|i| i.kind).collect();
        assert!(kinds.contains(&BackupKind::PreRestore), "{kinds:?}");
        assert_eq!(temp_files_left(&h), 0);
        h.service.create_backup().await.unwrap();
    }

    #[tokio::test]
    async fn test_restore_latest_without_regular_backups() {
        let empty = harness();
        let only_safety = harness();
        only_safety.storage.put(
            info("safety", 1, BackupFormat::PgCustom, BackupKind::PreRestore),
            b"x",
        );

        for h in [empty, only_safety] {
            let err = h
                .service
                .restore(&BackupTarget::Latest, "tracker")
                .await
                .unwrap_err();

            assert!(matches!(err, Error::NotFound(_)), "{err:?}");
            assert!(events(&h).is_empty(), "{:?}", events(&h));
        }
    }

    #[tokio::test]
    async fn test_writes_are_rejected_while_restore_runs() {
        let gate = Arc::new(Notify::new());
        let g = gate.clone();
        let h = harness_with(None, move |db| db.dump_gate = Some(g));
        put_good_backup(&h);

        let restore = tokio::spawn({
            let service = h.service.clone();
            async move {
                service
                    .restore(&BackupTarget::Id("good".into()), "tracker")
                    .await
            }
        });
        wait_until_locked(&h).await;

        let backup = h.service.create_backup().await;
        let second_restore = h.service.restore(&BackupTarget::Latest, "tracker").await;
        assert!(matches!(backup, Err(Error::Busy(_))), "{backup:?}");
        assert!(
            matches!(second_restore, Err(Error::Busy(_))),
            "{second_restore:?}"
        );

        gate.notify_one();
        restore.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn test_backup_survives_failed_retention_list() {
        let h = harness_with(Some(1), |_| {});
        h.storage.fail_list.store(true, Ordering::SeqCst);

        h.service.create_backup().await.unwrap();

        assert_eq!(h.storage.delete_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_backup_survives_failed_retention_deletes() {
        let h = harness_with(Some(1), |_| {});
        for (id, day) in [("old1", 1), ("old2", 2)] {
            h.storage.put(
                info(id, day, BackupFormat::PgCustom, BackupKind::Regular),
                b"x",
            );
        }
        h.storage.fail_delete.store(true, Ordering::SeqCst);

        h.service.create_backup().await.unwrap();

        assert_eq!(h.storage.delete_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_retention_reads_every_page() {
        let h = harness_with(Some(2), |_| {});
        let base = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        for i in 0..250 {
            let mut backup = info(
                &format!("b{i}"),
                1,
                BackupFormat::PgCustom,
                BackupKind::Regular,
            );
            backup.created_at = base + Duration::minutes(i);
            h.storage.put(backup, b"x");
        }

        h.service.create_backup().await.unwrap();

        // 250 old ones plus the new one, minus the 2 kept.
        assert_eq!(h.storage.deleted.lock().unwrap().len(), 249);
        let kept: Vec<String> = h.storage.infos().into_iter().map(|i| i.id).collect();
        assert_eq!(kept, ["up250", "b249"]);
    }

    /// Real pg tools, in-memory storage. Needs PostgreSQL 18, see backup-pg tests.
    #[tokio::test]
    #[ignore = "requires PostgreSQL 18 server and client binaries"]
    async fn test_backup_and_restore_with_postgres() {
        use backup_core::{DbTarget, Secret};
        use backup_pg::{PgTools, TokioCommandRunner, ZstdLevel};

        let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
        let target = |name: &str| DbTarget {
            host: env("PG_TEST_HOST", "localhost"),
            port: env("PG_TEST_PORT", "5432").parse().unwrap(),
            user: env("PG_TEST_USER", "postgres"),
            password: Secret::new(env("PG_TEST_PASSWORD", "postgres")),
            name: name.to_string(),
        };
        let tools = |name: &str| {
            Arc::new(PgTools::new(
                target(name),
                ZstdLevel::default(),
                std::time::Duration::from_secs(300),
                Arc::new(TokioCommandRunner),
            ))
        };
        let admin = tools("postgres");
        admin
            .query("DROP DATABASE IF EXISTS backup_service_test WITH (FORCE)")
            .await
            .unwrap();
        admin
            .query("CREATE DATABASE backup_service_test")
            .await
            .unwrap();
        let pg = tools("backup_service_test");
        pg.query("CREATE TABLE notes (id serial PRIMARY KEY, body text); INSERT INTO notes (body) VALUES ('a'), ('b')")
            .await
            .unwrap();

        let storage = Arc::new(FakeStorage::default());
        let dir = tempfile::tempdir().unwrap();
        let service = BackupService::new(ServiceDeps {
            dumper: pg.clone(),
            restorer: pg.clone(),
            health: pg.clone(),
            storage: storage.clone(),
            clock: Arc::new(FixedClock(Utc::now())),
            data_folder: dir.path().to_owned(),
            retention: None,
        });

        let backup = service.create_backup().await.unwrap();
        pg.query("INSERT INTO notes (body) VALUES ('after backup')")
            .await
            .unwrap();
        let outcome = service
            .restore(&BackupTarget::Id(backup.id.clone()), "backup_service_test")
            .await
            .unwrap();

        let bodies = pg
            .query("SELECT string_agg(body, ',' ORDER BY id) FROM notes")
            .await
            .unwrap();
        assert_eq!(bodies.trim(), "a,b");
        assert_eq!(outcome.pre_restore.kind, BackupKind::PreRestore);

        let safety = service
            .restore(
                &BackupTarget::Id(outcome.pre_restore.id.clone()),
                "backup_service_test",
            )
            .await
            .unwrap();
        let bodies = pg
            .query("SELECT string_agg(body, ',' ORDER BY id) FROM notes")
            .await
            .unwrap();
        assert_eq!(bodies.trim(), "a,b,after backup");
        assert_eq!(safety.restored.id, outcome.pre_restore.id);
    }

    #[tokio::test]
    async fn test_healthcheck() {
        assert!(harness().service.healthcheck().await);
        assert!(
            !harness_with(None, |db| db.healthy = false)
                .service
                .healthcheck()
                .await
        );
    }
}

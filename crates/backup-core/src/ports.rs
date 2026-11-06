use std::path::Path;

use crate::{BackupInfo, Page, Result, UploadRequest};

#[async_trait::async_trait]
pub trait Dumper: Send + Sync {
    async fn dump(&self, dst: &Path) -> Result<()>;
}

#[async_trait::async_trait]
pub trait Restorer: Send + Sync {
    async fn restore(&self, src: &Path) -> Result<()>;
}

/// Backup files in one storage location.
#[async_trait::async_trait]
pub trait Storage: Send + Sync {
    async fn upload(&self, req: UploadRequest<'_>) -> Result<BackupInfo>;

    /// Newest first.
    async fn list(&self, page_size: u32, page_token: Option<&str>) -> Result<Page<BackupInfo>>;

    /// Newest backup that is not a pre-restore copy.
    async fn latest(&self) -> Result<Option<BackupInfo>>;

    /// Fails with `NotFound` for files outside the storage location.
    async fn get(&self, id: &str) -> Result<BackupInfo>;

    /// Writes the content to `dst`, verifying the checksum when the storage has one.
    async fn download(&self, backup: &BackupInfo, dst: &Path) -> Result<()>;

    async fn delete(&self, id: &str) -> Result<()>;
}

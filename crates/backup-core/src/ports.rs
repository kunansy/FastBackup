use std::path::Path;

use crate::Result;

#[async_trait::async_trait]
pub trait Dumper: Send + Sync {
    async fn dump(&self, dst: &Path) -> Result<()>;
}

#[async_trait::async_trait]
pub trait Restorer: Send + Sync {
    async fn restore(&self, src: &Path) -> Result<()>;
}

#[async_trait::async_trait]
pub trait Storage: Send + Sync {
    async fn upload(&self, buf: &[u8], filename: &str, folder_id: Option<String>)
    -> Result<String>;

    async fn download(&self, folder_id: &str) -> Result<(Vec<u8>, String)>;
}

pub mod compression;
pub mod errors;

pub use errors::Errors;

pub type Res<T> = Result<T, Errors>;

#[async_trait::async_trait]
pub trait Storage {
    async fn upload(&self, buf: &[u8], filename: &str, folder_id: Option<String>) -> Res<String>;

    async fn download(&self, folder_id: &str) -> Res<(Vec<u8>, String)>;
}

pub trait DbConfig {
    fn db_host(&self) -> &str;
    fn db_port(&self) -> u16;
    fn db_username(&self) -> &str;
    fn db_password(&self) -> &str;
    fn db_name(&self) -> &str;
}

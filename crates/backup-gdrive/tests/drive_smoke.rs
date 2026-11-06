//! Talks to the real Google Drive.
//! Run: `DRIVE_TEST_CREDS=$(cat key.json) DRIVE_TEST_FOLDER_ID=... cargo test -p backup-gdrive -- --ignored`.

use backup_core::{BackupFormat, BackupKind, Storage, UploadRequest};
use backup_gdrive::{DriveOptions, GoogleDriveStorage};

#[tokio::test]
#[ignore = "requires Google Drive service account and a test folder"]
async fn test_upload_find_download_delete() {
    let creds = std::env::var("DRIVE_TEST_CREDS").expect("DRIVE_TEST_CREDS");
    let folder = std::env::var("DRIVE_TEST_FOLDER_ID").expect("DRIVE_TEST_FOLDER_ID");
    let drive = GoogleDriveStorage::connect(&creds, folder, DriveOptions::default())
        .await
        .expect("connected");

    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src.pgdump");
    let content = b"smoke test content ".repeat(1000);
    std::fs::write(&src, &content).unwrap();

    let uploaded = drive
        .upload(UploadRequest {
            path: &src,
            name: "smoke_test.pgdump",
            db_name: "smoke",
            format: BackupFormat::PgCustom,
            kind: BackupKind::Regular,
        })
        .await
        .expect("uploaded");
    assert_eq!(uploaded.size_bytes, Some(content.len() as u64));

    let latest = drive.latest().await.expect("latest").expect("some backup");
    assert_eq!(latest.id, uploaded.id);

    let fetched = drive.get(&uploaded.id).await.expect("get");
    assert_eq!(fetched.format, BackupFormat::PgCustom);

    let dst = dir.path().join("dst.pgdump");
    drive.download(&fetched, &dst).await.expect("downloaded");
    assert_eq!(std::fs::read(&dst).unwrap(), content);

    drive.delete(&uploaded.id).await.expect("deleted");
    assert!(drive.get(&uploaded.id).await.is_err());
}

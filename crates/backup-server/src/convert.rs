use backup_core::{BackupFormat, BackupInfo, BackupKind};
use backup_proto as proto;
use tonic::Status;

use crate::service::BackupTarget;

pub fn backup_info(info: &BackupInfo) -> proto::BackupInfo {
    let format = match info.format {
        BackupFormat::PgCustom => proto::BackupFormat::PgCustom,
        BackupFormat::LegacyJsonZstd => proto::BackupFormat::LegacyJsonZstd,
    };
    let kind = match info.kind {
        BackupKind::Regular => proto::BackupKind::Regular,
        BackupKind::PreRestore => proto::BackupKind::PreRestore,
    };

    proto::BackupInfo {
        id: info.id.clone(),
        name: info.name.clone(),
        created_at: Some(prost_types::Timestamp {
            seconds: info.created_at.timestamp(),
            nanos: i32::try_from(info.created_at.timestamp_subsec_nanos()).unwrap_or(0),
        }),
        size_bytes: info.size_bytes,
        format: format.into(),
        kind: kind.into(),
    }
}

pub fn backup_target(backup: Option<proto::BackupRef>) -> Result<BackupTarget, Status> {
    use proto::backup_ref::Target;

    match backup.and_then(|b| b.target) {
        Some(Target::Id(id)) if !id.is_empty() => Ok(BackupTarget::Id(id)),
        Some(Target::Latest(true)) => Ok(BackupTarget::Latest),
        _ => Err(Status::invalid_argument(
            "backup must set a non-empty id or latest = true",
        )),
    }
}

#[cfg(test)]
mod tests {
    use backup_core::{BackupFormat, BackupKind};
    use backup_proto as proto;
    use backup_proto::backup_ref::Target;
    use tonic::Code;

    use super::{backup_info, backup_target};
    use crate::service::BackupTarget;
    use crate::service::tests::info;

    #[test]
    fn test_backup_info() {
        let converted = backup_info(&info(
            "id1",
            2,
            BackupFormat::LegacyJsonZstd,
            BackupKind::PreRestore,
        ));

        assert_eq!(converted.id, "id1");
        assert_eq!(converted.format(), proto::BackupFormat::LegacyJsonZstd);
        assert_eq!(converted.kind(), proto::BackupKind::PreRestore);
        assert_eq!(converted.created_at.unwrap().seconds, 1_767_312_000);
    }

    #[test]
    fn test_backup_target() {
        let target = |t| backup_target(Some(proto::BackupRef { target: t }));

        assert_eq!(
            target(Some(Target::Id("x".into()))).unwrap(),
            BackupTarget::Id("x".into())
        );
        assert_eq!(
            target(Some(Target::Latest(true))).unwrap(),
            BackupTarget::Latest
        );
        for bad in [
            None,
            Some(Target::Id(String::new())),
            Some(Target::Latest(false)),
        ] {
            assert_eq!(target(bad).unwrap_err().code(), Code::InvalidArgument);
        }
        assert_eq!(
            backup_target(None).unwrap_err().code(),
            Code::InvalidArgument
        );
    }
}

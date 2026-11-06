use std::fmt;
use std::ops::RangeInclusive;

use backup_core::{Error, Result};

const LEVELS: RangeInclusive<u8> = 1..=22;
const DEFAULT_LEVEL: u8 = 3;

/// zstd level accepted by `pg_dump --compress=zstd:<level>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZstdLevel(u8);

impl ZstdLevel {
    pub fn new(level: i64) -> Result<Self> {
        u8::try_from(level)
            .ok()
            .filter(|l| LEVELS.contains(l))
            .map(Self)
            .ok_or_else(|| {
                Error::Config(format!("zstd level must be in {LEVELS:?}, {level} found"))
            })
    }
}

impl Default for ZstdLevel {
    fn default() -> Self {
        Self(DEFAULT_LEVEL)
    }
}

impl fmt::Display for ZstdLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::ZstdLevel;

    #[test]
    fn test_accepts_bounds() {
        assert!(ZstdLevel::new(1).is_ok());
        assert!(ZstdLevel::new(22).is_ok());
    }

    #[test]
    fn test_rejects_out_of_range() {
        for level in [-1, 0, 23, 300] {
            assert!(ZstdLevel::new(level).is_err(), "{level}");
        }
    }
}

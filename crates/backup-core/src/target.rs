use std::fmt;

#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbTarget {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: Secret,
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::{DbTarget, Secret};

    #[test]
    fn test_debug_does_not_leak_password() {
        let target = DbTarget {
            host: "db".into(),
            port: 5432,
            user: "u".into(),
            password: Secret::new("hunter2"),
            name: "tracker".into(),
        };

        let printed = format!("{target:?}");

        assert!(!printed.contains("hunter2"), "{printed}");
        assert!(printed.contains("Secret(***)"), "{printed}");
    }
}

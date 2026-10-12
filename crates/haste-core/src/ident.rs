use std::fmt;

use serde::{Deserialize, Serialize};

/// A namespaced identifier like `minecraft:stone`. Namespace defaults to `minecraft`.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Identifier {
    namespace: String,
    path: String,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid identifier {0:?}")]
pub struct InvalidIdentifier(pub String);

impl Identifier {
    pub fn new(namespace: &str, path: &str) -> Self {
        Self {
            namespace: namespace.to_owned(),
            path: path.to_owned(),
        }
    }
    pub fn minecraft(path: &str) -> Self {
        Self::new("minecraft", path)
    }
    /// Parses `ns:path` or `path` (namespace `minecraft`). Allowed chars: `[a-z0-9_.-]` for the
    /// namespace and additionally `/` for the path.
    pub fn parse(s: &str) -> Result<Self, InvalidIdentifier> {
        let (ns, path) = match s.split_once(':') {
            Some((ns, p)) => (ns, p),
            None => ("minecraft", s),
        };
        let ok_ns = !ns.is_empty()
            && ns
                .bytes()
                .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-'));
        let ok_path = !path.is_empty()
            && path
                .bytes()
                .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'/'));
        if ok_ns && ok_path {
            Ok(Self::new(ns, path))
        } else {
            Err(InvalidIdentifier(s.to_owned()))
        }
    }
    pub fn namespace(&self) -> &str {
        &self.namespace
    }
    pub fn path(&self) -> &str {
        &self.path
    }
}

impl fmt::Display for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.namespace, self.path)
    }
}
impl fmt::Debug for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}
impl TryFrom<String> for Identifier {
    type Error = InvalidIdentifier;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::parse(&s)
    }
}
impl From<Identifier> for String {
    fn from(i: Identifier) -> String {
        i.to_string()
    }
}
impl std::str::FromStr for Identifier {
    type Err = InvalidIdentifier;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

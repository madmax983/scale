//! Typed ids. Use these types, not raw strings.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::MeshError;

/// The maximum length of an id.
pub const MAX_ID_LEN: usize = 64;

/// Returns `true` if `s` is a valid id: 1-64 characters `a-z`, `0-9`, `.`, `_`, `-`,
/// and the first character is `a-z` or `0-9`.
#[must_use]
pub fn is_valid_id(s: &str) -> bool {
    let b = s.as_bytes();
    let body =
        |c: &u8| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'_' | b'-');
    match b.first() {
        Some(first) => {
            b.len() <= MAX_ID_LEN
                && (first.is_ascii_lowercase() || first.is_ascii_digit())
                && b.iter().all(body)
        }
        None => false,
    }
}

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Makes the id. Returns an error if the text is not a valid id.
            ///
            /// # Errors
            /// [`MeshError::InvalidId`] if the text fails [`is_valid_id`].
            pub fn new(s: impl Into<String>) -> Result<Self, MeshError> {
                let s = s.into();
                if is_valid_id(&s) {
                    Ok(Self(s))
                } else {
                    Err(MeshError::InvalidId(s))
                }
            }

            /// Returns the id text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = MeshError;
            fn try_from(s: String) -> Result<Self, MeshError> {
                Self::new(s)
            }
        }

        impl From<$name> for String {
            fn from(v: $name) -> Self {
                v.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = MeshError;
            fn from_str(s: &str) -> Result<Self, MeshError> {
                Self::new(s)
            }
        }
    };
}

id_type!(
    /// A task id, for example `spec-1370-build`.
    TaskId
);
id_type!(
    /// An agent (role) id, for example `builder`.
    AgentId
);
id_type!(
    /// A skill tag, for example `build`.
    Skill
);

/// A fencing token. Each claim gets a larger token than all earlier claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Token(pub u64);

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Token {
    type Err = MeshError;
    fn from_str(s: &str) -> Result<Self, MeshError> {
        s.parse::<u64>()
            .ok()
            .filter(|n| *n > 0)
            .map(Self)
            .ok_or_else(|| MeshError::InvalidToken(s.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_ids() {
        for s in [
            "a",
            "0",
            "spec-1370-build",
            "lore-master",
            "a.b_c-d",
            &"x".repeat(64),
        ] {
            assert!(is_valid_id(s), "{s}");
        }
    }

    #[test]
    fn rejects_invalid_ids() {
        for s in [
            "",
            "-a",
            ".a",
            "_a",
            "A",
            "a b",
            "a/b",
            "a\n",
            "é",
            &"x".repeat(65),
        ] {
            assert!(!is_valid_id(s), "{s:?}");
        }
    }

    #[test]
    fn typed_ids_validate_on_new_and_parse() {
        assert!(TaskId::new("ok-1").is_ok());
        assert!(matches!(AgentId::new("Bad"), Err(MeshError::InvalidId(_))));
        assert!("build".parse::<Skill>().is_ok());
    }

    #[test]
    fn typed_ids_validate_on_deserialize() {
        let ok: Result<TaskId, _> = serde_json::from_str("\"t-1\"");
        assert!(ok.is_ok());
        let bad: Result<TaskId, _> = serde_json::from_str("\"../x\"");
        assert!(bad.is_err());
    }

    #[test]
    fn token_parses_positive_integers_only() {
        assert_eq!("7".parse::<Token>().ok(), Some(Token(7)));
        assert!("0".parse::<Token>().is_err());
        assert!("-1".parse::<Token>().is_err());
        assert!("x".parse::<Token>().is_err());
        assert_eq!(Token(9).to_string(), "9");
    }
}

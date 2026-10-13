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
    b.len() <= MAX_ID_LEN
        && b.first()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && b.iter().all(body)
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
                f.pad(&self.0)
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

/// A fencing token: a counter and a random nonce, written `7-9f3a2b1c`.
///
/// Each claim gets a larger counter than all earlier claims in the ledger.
/// The nonce keeps tokens unique when a ledger is rolled back or merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Token {
    /// The counter.
    pub n: u64,
    /// The nonce.
    pub nonce: u32,
}

impl Token {
    /// Makes a token.
    #[must_use]
    pub const fn new(n: u64, nonce: u32) -> Self {
        Self { n, nonce }
    }
}

impl TryFrom<String> for Token {
    type Error = MeshError;
    fn try_from(s: String) -> Result<Self, MeshError> {
        s.parse()
    }
}

impl From<Token> for String {
    fn from(t: Token) -> Self {
        t.to_string()
    }
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(&format!("{}-{:08x}", self.n, self.nonce))
    }
}

/// A new random nonce for a token. It is unique enough to tell claims apart; it is not a secret.
#[must_use]
pub fn fresh_nonce() -> u32 {
    use std::hash::BuildHasher;
    let seed = (std::time::SystemTime::now(), std::process::id());
    let hash = std::collections::hash_map::RandomState::new().hash_one(seed);
    let [a, b, c, d, ..] = hash.to_le_bytes();
    u32::from_le_bytes([a, b, c, d])
}

impl FromStr for Token {
    type Err = MeshError;
    fn from_str(s: &str) -> Result<Self, MeshError> {
        let bad = || MeshError::InvalidToken(s.to_string());
        let (n, nonce) = s.split_once('-').unwrap_or((s, "0"));
        let n = n.parse::<u64>().ok().filter(|n| *n > 0).ok_or_else(bad)?;
        let hex =
            !nonce.is_empty() && nonce.len() <= 8 && nonce.bytes().all(|b| b.is_ascii_hexdigit());
        let nonce = u32::from_str_radix(nonce, 16)
            .ok()
            .filter(|_| hex)
            .ok_or_else(bad)?;
        Ok(Self { n, nonce })
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
        assert_eq!("7".parse::<Token>().ok(), Some(Token::new(7, 0)));
        assert_eq!("7-0000002a".parse::<Token>().ok(), Some(Token::new(7, 42)));
        assert!("7-xyz".parse::<Token>().is_err());
        assert!("7-123456789".parse::<Token>().is_err());
        assert!("0".parse::<Token>().is_err());
        assert!("-1".parse::<Token>().is_err());
        assert!("x".parse::<Token>().is_err());
        assert_eq!(Token::new(9, 0xab).to_string(), "9-000000ab");
    }

    #[test]
    fn display_honors_width() {
        let id = TaskId::new("ab").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(format!("[{id:<4}]"), "[ab  ]");
        assert_eq!(format!("[{:>12}]", Token::new(7, 0)), "[  7-00000000]");
    }
}

//! The registry of agent cards (`.mesh/registry.toml`).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::error::{MeshError, Result};
use crate::event::{validate_text, MAX_TITLE};
use crate::ids::{AgentId, Skill};
use crate::scope::Scope;

/// The registry format version.
pub const REGISTRY_VERSION: u32 = 1;
/// The largest `max_active` value.
pub const MAX_ACTIVE: u32 = 16;
/// The registry for SCALE. `mesh init` writes it.
pub const DEFAULT_REGISTRY: &str = include_str!("default_registry.toml");

const fn one() -> u32 {
    1
}

/// One agent card. The `id` is the Claude Code subagent name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCard {
    /// Role id.
    pub id: AgentId,
    /// One-line purpose.
    pub description: String,
    /// Task skills that the role can do.
    pub skills: Vec<Skill>,
    /// Path globs that the role may change.
    #[serde(default)]
    pub writes: Vec<String>,
    /// The most working tasks at one time.
    #[serde(default = "one")]
    pub max_active: u32,
}

impl AgentCard {
    /// `true` if the card has the skill.
    #[must_use]
    pub fn has_skill(&self, skill: &Skill) -> bool {
        self.skills.contains(skill)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryFile {
    version: u32,
    #[serde(default, rename = "agent")]
    agents: Vec<AgentCard>,
}

/// A checked registry with compiled scopes.
#[derive(Debug, Clone)]
pub struct Registry {
    cards: Vec<AgentCard>,
    scopes: Vec<Scope>,
}

impl Registry {
    /// Parses and checks registry TOML.
    ///
    /// # Errors
    /// [`MeshError::Registry`] or [`MeshError::InvalidPattern`] for the first problem.
    pub fn parse(text: &str) -> Result<Self> {
        let file: RegistryFile =
            toml::from_str(text).map_err(|e| MeshError::Registry(e.to_string()))?;
        if file.version != REGISTRY_VERSION {
            return Err(MeshError::Registry(format!(
                "unsupported version {}; use version = {REGISTRY_VERSION}",
                file.version
            )));
        }
        if file.agents.is_empty() {
            return Err(MeshError::Registry(
                "no agent; add an [[agent]] table".into(),
            ));
        }
        let mut seen = BTreeSet::new();
        let mut scopes = Vec::with_capacity(file.agents.len());
        for card in &file.agents {
            let fail = |msg: String| MeshError::Registry(format!("agent `{}`: {msg}", card.id));
            if !seen.insert(&card.id) {
                return Err(fail("duplicate id".into()));
            }
            validate_text("description", &card.description, MAX_TITLE, true, true)
                .map_err(|e| fail(e.to_string()))?;
            if card.skills.is_empty() {
                return Err(fail("add at least one skill".into()));
            }
            if !(1..=MAX_ACTIVE).contains(&card.max_active) {
                return Err(fail(format!("max_active must be 1..={MAX_ACTIVE}")));
            }
            scopes.push(Scope::new(&card.writes)?);
        }
        Ok(Self {
            cards: file.agents,
            scopes,
        })
    }

    /// All cards, in file order.
    #[must_use]
    pub fn cards(&self) -> &[AgentCard] {
        &self.cards
    }

    /// The card with this id.
    #[must_use]
    pub fn get(&self, id: &AgentId) -> Option<&AgentCard> {
        self.cards.iter().find(|c| &c.id == id)
    }

    /// The compiled write scope of the role.
    #[must_use]
    pub fn scope(&self, id: &AgentId) -> Option<&Scope> {
        let i = self.cards.iter().position(|c| &c.id == id)?;
        self.scopes.get(i)
    }

    /// `true` if at least one card has the skill.
    #[must_use]
    pub fn serves(&self, skill: &Skill) -> bool {
        self.cards.iter().any(|c| c.has_skill(skill))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aid(s: &str) -> AgentId {
        AgentId::new(s).unwrap_or_else(|e| panic!("{e}"))
    }

    fn parse_err(text: &str) -> String {
        match Registry::parse(text) {
            Ok(_) => panic!("expected an error for:\n{text}"),
            Err(e) => e.to_string(),
        }
    }

    #[test]
    fn default_registry_has_the_seven_scale_roles() {
        let r = Registry::parse(DEFAULT_REGISTRY).unwrap_or_else(|e| panic!("{e}"));
        let ids: Vec<&str> = r.cards().iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "designer",
                "lore-master",
                "architect",
                "builder",
                "integrator",
                "codex",
                "reviewer"
            ]
        );
        let builder = r.get(&aid("builder")).unwrap_or_else(|| panic!("builder"));
        assert!(builder.has_skill(&Skill::new("build").unwrap_or_else(|e| panic!("{e}"))));
        assert_eq!(builder.max_active, 3);
        let scope = r
            .scope(&aid("architect"))
            .unwrap_or_else(|| panic!("scope"));
        assert!(scope.is_match("specs/1384-x.md"));
        assert!(!scope.is_match("src/lib.rs"));
        let reviewer = r.scope(&aid("reviewer")).unwrap_or_else(|| panic!("scope"));
        assert!(!reviewer.is_match("src/lib.rs"));
        assert!(r.serves(&Skill::new("review").unwrap_or_else(|e| panic!("{e}"))));
        assert!(!r.serves(&Skill::new("dance").unwrap_or_else(|e| panic!("{e}"))));
        assert!(r.get(&aid("nobody")).is_none());
    }

    #[test]
    fn rejects_bad_registries() {
        let card = |id: &str, skills: &str, writes: &str, max: u32| {
            format!(
                "[[agent]]\nid = \"{id}\"\ndescription = \"d\"\nskills = [{skills}]\nwrites = [{writes}]\nmax_active = {max}\n"
            )
        };
        let ok = card("a", "\"x\"", "\"src/**\"", 1);
        assert!(Registry::parse(&format!("version = 1\n{ok}")).is_ok());

        assert!(parse_err(&format!("version = 2\n{ok}")).contains("version"));
        assert!(parse_err("version = 1\n").contains("no agent"));
        assert!(parse_err(&format!("version = 1\n{ok}{ok}")).contains("duplicate"));
        assert!(
            parse_err(&format!("version = 1\n{}", card("Bad", "\"x\"", "", 1)))
                .contains("invalid id")
        );
        assert!(parse_err(&format!("version = 1\n{}", card("a", "", "", 1))).contains("skill"));
        assert!(parse_err(&format!(
            "version = 1\n{}",
            card("a", "\"x\"", "\"../x\"", 1)
        ))
        .contains("pattern"));
        assert!(
            parse_err(&format!("version = 1\n{}", card("a", "\"x\"", "", 0)))
                .contains("max_active")
        );
        assert!(parse_err(&format!(
            "version = 1\n{}",
            card("a", "\"x\"", "", MAX_ACTIVE + 1)
        ))
        .contains("max_active"));
        assert!(parse_err(
            "version = 1\n[[agent]]\nid = \"a\"\ndescription = \"\"\nskills = [\"x\"]\n"
        )
        .contains("description"));
        assert!(parse_err("version = 1\nextra = 1\n").contains("registry"));
        assert!(parse_err("not toml [").contains("registry"));
    }
}

//! Plan files: many tasks added in one atomic step.
//!
//! Format (TOML): one `[[task]]` table per task, with the [`TaskSpec`] fields.

use serde::Deserialize;

use crate::error::{MeshError, Result};
use crate::event::TaskSpec;
use crate::ledger::Ledger;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanFile {
    #[serde(default, rename = "task")]
    tasks: Vec<TaskSpec>,
}

/// Parses plan TOML.
///
/// # Errors
/// [`MeshError::Plan`] if the TOML is not valid or has no task.
pub fn parse(text: &str) -> Result<Vec<TaskSpec>> {
    let _ = (text, toml::from_str::<PlanFile>);
    todo!("RED")
}

/// Checks the plan against the ledger and sorts it so each task comes after its dependencies.
/// The sort keeps the file order where it can.
///
/// # Errors
/// [`MeshError::DuplicateTask`], [`MeshError::UnknownDependency`], or
/// [`MeshError::Cycle`] with the cycle path.
pub fn order(ledger: &Ledger, specs: Vec<TaskSpec>) -> Result<Vec<TaskSpec>> {
    let _ = (ledger, specs);
    todo!("RED")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::TaskId;

    fn ids(v: &[TaskSpec]) -> Vec<&str> {
        v.iter().map(|t| t.id.as_str()).collect()
    }

    fn tid(s: &str) -> TaskId {
        TaskId::new(s).unwrap_or_else(|e| panic!("{e}"))
    }

    const PLAN: &str = r#"
[[task]]
id = "review"
title = "Review"
skill = "review"
deps = ["build"]

[[task]]
id = "spec"
title = "Spec"
skill = "spec"

[[task]]
id = "build"
title = "Build"
skill = "build"
deps = ["spec"]
paths = ["src/**"]
max_failures = 2
"#;

    #[test]
    fn parses_and_orders_by_dependencies() {
        let specs = parse(PLAN).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(ids(&specs), ["review", "spec", "build"]);
        assert_eq!(specs[2].max_failures, 2);
        let ordered = order(&Ledger::new(), specs).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(ids(&ordered), ["spec", "build", "review"]);
    }

    #[test]
    fn rejects_empty_and_invalid_plans() {
        assert!(matches!(parse(""), Err(MeshError::Plan(_))));
        assert!(matches!(parse("[[task]]\nid = \"a\"\n"), Err(MeshError::Plan(_))));
        assert!(matches!(parse("[[tasks]]\n"), Err(MeshError::Plan(_))));
    }

    #[test]
    fn rejects_duplicates_and_unknown_deps() {
        let a = TaskSpec::new(tid("a"), "A", crate::ids::Skill::new("x").unwrap_or_else(|e| panic!("{e}")));
        let dup = order(&Ledger::new(), vec![a.clone(), a.clone()]);
        assert!(matches!(dup, Err(MeshError::DuplicateTask(_))));

        let mut b = a.clone();
        b.id = tid("b");
        b.deps = vec![tid("zzz")];
        let unknown = order(&Ledger::new(), vec![a, b]);
        assert!(matches!(unknown, Err(MeshError::UnknownDependency { .. })));
    }

    #[test]
    fn reports_the_cycle_path() {
        let skill = crate::ids::Skill::new("x").unwrap_or_else(|e| panic!("{e}"));
        let mut a = TaskSpec::new(tid("a"), "A", skill.clone());
        let mut b = TaskSpec::new(tid("b"), "B", skill.clone());
        let mut c = TaskSpec::new(tid("c"), "C", skill);
        a.deps = vec![tid("c")];
        b.deps = vec![tid("a")];
        c.deps = vec![tid("b")];
        match order(&Ledger::new(), vec![a, b, c]) {
            Err(MeshError::Cycle(path)) => {
                assert_eq!(path.first(), path.last());
                assert_eq!(path.len(), 4);
            }
            other => panic!("expected a cycle, got {other:?}"),
        }
    }

    #[test]
    fn self_dependency_is_a_cycle() {
        let mut a = TaskSpec::new(tid("a"), "A", crate::ids::Skill::new("x").unwrap_or_else(|e| panic!("{e}")));
        a.deps = vec![tid("a")];
        assert!(matches!(order(&Ledger::new(), vec![a]), Err(MeshError::Cycle(_))));
    }
}

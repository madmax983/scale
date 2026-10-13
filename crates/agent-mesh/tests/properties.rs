//! Property tests: invariants I1-I9 hold for random command sequences.
//! I1-I5 are also proved in `verus/lifecycle.rs`.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use agent_mesh::event::{Envelope, Event, TaskSpec};
use agent_mesh::ids::{AgentId, Skill, TaskId, Token};
use agent_mesh::ledger::{Command, Ctx, Ledger, NoteKind};
use agent_mesh::lifecycle::State;
use agent_mesh::registry::{Registry, DEFAULT_REGISTRY};
use agent_mesh::{plan, store, MeshError};
use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};

const SKILLS: [&str; 3] = ["build", "review", "spec"];
/// The role for each skill above, in the same order.
const ROLES: [&str; 3] = ["builder", "reviewer", "architect"];
const DETAILS: [&str; 4] = ["", "Read specs/1.md.", "line 1\nline 2", "café ✓"];

#[derive(Debug, Clone)]
enum Op {
    Claim(usize),
    ClaimAs(usize, usize),
    ClaimNext(usize),
    Heartbeat(usize, bool),
    Complete(usize, bool, bool),
    Fail(usize, bool),
    Ask(usize, bool),
    Answer(usize),
    Retry(usize),
    Cancel(usize),
    Reap,
    Wait(u64),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        5 => (0..6usize).prop_map(Op::Claim),
        1 => (0..6usize, 0..3usize).prop_map(|(t, a)| Op::ClaimAs(t, a)),
        2 => (0..3usize).prop_map(Op::ClaimNext),
        2 => (0..6usize, any::<bool>()).prop_map(|(t, s)| Op::Heartbeat(t, s)),
        4 => (0..6usize, any::<bool>(), any::<bool>()).prop_map(|(t, s, e)| Op::Complete(t, s, e)),
        3 => (0..6usize, any::<bool>()).prop_map(|(t, s)| Op::Fail(t, s)),
        2 => (0..6usize, any::<bool>()).prop_map(|(t, s)| Op::Ask(t, s)),
        2 => (0..6usize).prop_map(Op::Answer),
        3 => (0..6usize).prop_map(Op::Retry),
        1 => (0..6usize).prop_map(Op::Cancel),
        1 => Just(Op::Reap),
        2 => (1..90u64).prop_map(Op::Wait),
    ]
}

/// Six tasks. Each task may depend on earlier tasks only.
fn tasks() -> impl Strategy<Value = Vec<TaskSpec>> {
    let row = (
        0..3usize,
        1..4u32,
        prop::collection::vec(any::<bool>(), 6),
        0..4usize,
        any::<bool>(),
    );
    prop::collection::vec(row, 6).prop_map(|rows| {
        rows.into_iter()
            .enumerate()
            .map(|(i, (skill, budget, dep_bits, detail, paths))| {
                let skill = Skill::new(SKILLS[skill]).unwrap_or_else(|e| panic!("{e}"));
                let mut s = TaskSpec::new(tid(i), format!("Task {i}"), skill);
                s.max_failures = budget;
                s.deps = (0..i).filter(|j| dep_bits[*j]).map(tid).collect();
                s.detail = DETAILS[detail].to_string();
                if paths {
                    s.paths = vec!["src/**".into()];
                }
                s
            })
            .collect()
    })
}

fn tid(i: usize) -> TaskId {
    TaskId::new(format!("t{i}")).unwrap_or_else(|e| panic!("{e}"))
}

fn role(i: usize) -> AgentId {
    AgentId::new(ROLES[i]).unwrap_or_else(|e| panic!("{e}"))
}

/// The role that has the skill of task `t`.
fn role_for(l: &Ledger, t: usize) -> AgentId {
    let skill = l.task(&tid(t)).map_or("build", |x| x.spec.skill.as_str());
    role(SKILLS.iter().position(|s| *s == skill).unwrap_or(0))
}

/// Task `t` if it is working, else the first working task, else `t`.
fn working(l: &Ledger, t: usize) -> usize {
    let is_working = |i: usize| {
        l.task(&tid(i))
            .is_some_and(|x| x.life.state() == State::Working)
    };
    if is_working(t) {
        t
    } else {
        (0..6).find(|i| is_working(*i)).unwrap_or(t)
    }
}

fn token_for(l: &Ledger, t: usize, stale: bool) -> Token {
    let current = l
        .task(&tid(t))
        .and_then(|x| x.life.lease())
        .map_or(Token::new(1, 0), |x| x.token);
    if stale {
        Token::new(current.n, current.nonce ^ 1)
    } else {
        current
    }
}

fn command(l: &Ledger, op: &Op) -> Option<Command> {
    let claim = |t: usize, agent: AgentId| Command::Claim {
        task: Some(tid(t)),
        agent,
        worker: Some(format!("w{t}")),
        ttl: 60,
    };
    Some(match *op {
        Op::Claim(t) => claim(t, role_for(l, t)),
        Op::ClaimAs(t, a) => claim(t, role(a)),
        Op::ClaimNext(a) => Command::Claim {
            task: None,
            agent: role(a),
            worker: None,
            ttl: 60,
        },
        Op::Heartbeat(t, s) => {
            let t = working(l, t);
            Command::Heartbeat {
                task: tid(t),
                token: token_for(l, t, s),
                ttl: 60,
            }
        }
        Op::Complete(t, s, empty) => {
            let t = working(l, t);
            let evidence = if empty {
                String::new()
            } else {
                "tests pass".into()
            };
            Command::Complete {
                task: tid(t),
                token: token_for(l, t, s),
                evidence,
            }
        }
        Op::Fail(t, s) => {
            let t = working(l, t);
            Command::Fail {
                task: tid(t),
                token: token_for(l, t, s),
                reason: "broken".into(),
            }
        }
        Op::Ask(t, s) => {
            let t = working(l, t);
            Command::Ask {
                task: tid(t),
                token: token_for(l, t, s),
                question: "why?".into(),
            }
        }
        Op::Answer(t) => Command::Answer {
            task: tid(t),
            text: "because".into(),
        },
        Op::Retry(t) => Command::Retry { task: tid(t) },
        Op::Cancel(t) => Command::Cancel {
            task: tid(t),
            reason: "stop".into(),
        },
        Op::Reap => Command::Reap,
        Op::Wait(_) => return None,
    })
}

/// Paths that the coverage test counts.
#[derive(Debug, Default)]
struct Seen {
    dependent_claims: usize,
    retries: usize,
    answers: usize,
    expiries: usize,
    budget_used: usize,
    wip_limits: usize,
    completions: usize,
}

/// Checks I1-I9 for one accepted step.
fn check_step(
    before: &Ledger,
    after: &Ledger,
    events: &[Envelope],
    now: u64,
    max_token: &mut u64,
    reg: &Registry,
) {
    for t in after.tasks() {
        // I1 and I5 (state part).
        assert!(t.life.invariant_holds(), "I1/I5 broken: {t:?}");
        assert!(t.life.failures() <= t.life.max_failures(), "I5 broken");
        // I8: completed tasks have evidence.
        if t.life.state() == State::Completed {
            assert!(
                t.last_note(NoteKind::Evidence)
                    .is_some_and(|e| !e.trim().is_empty()),
                "I8 broken"
            );
        }
        if let Some(old) = before.task(&t.spec.id) {
            // I2: terminal tasks do not change.
            if old.life.state().is_terminal() {
                assert_eq!(old.life, t.life, "I2 broken");
            }
            // I5: failed exits only by retry inside the budget, or cancel.
            if old.life.state() == State::Failed && t.life.state() != State::Failed {
                let retried = events
                    .iter()
                    .any(|e| matches!(&e.event, Event::Retried { task } if task == &t.spec.id));
                let canceled = t.life.state() == State::Canceled;
                assert!(
                    canceled || (retried && old.life.failures() < old.life.max_failures()),
                    "I5 exit broken"
                );
            }
        }
    }
    // I9: WIP limit per role.
    for c in reg.cards() {
        assert!(
            after.active_count(&c.id) <= c.max_active as usize,
            "I9 broken for {}",
            c.id
        );
    }
    for e in events {
        let task = e.event.task().and_then(|id| before.task(id));
        match &e.event {
            // I3: fresh tokens. I6: dependencies completed before the claim.
            Event::Claimed { token, .. } => {
                assert!(token.n > *max_token, "I3 broken");
                *max_token = token.n;
                for d in &task.unwrap_or_else(|| panic!("task")).spec.deps {
                    assert_eq!(
                        before.task(d).map(|x| x.life.state()),
                        Some(State::Completed),
                        "I6 broken"
                    );
                }
            }
            // I4: worker writes need the live lease with the same token.
            Event::Heartbeat { token, .. }
            | Event::Completed { token, .. }
            | Event::Failed { token, .. }
            | Event::InputRequested { token, .. } => {
                let lease = task
                    .and_then(|x| x.life.lease())
                    .unwrap_or_else(|| panic!("I4: no lease"));
                assert_eq!(lease.token, *token, "I4 token");
                assert!(now < lease.expires_at, "I4 expiry");
            }
            _ => {}
        }
        // I5: a failure and a lease expiry each count one failure.
        if matches!(e.event, Event::Failed { .. } | Event::LeaseExpired { .. }) {
            let id = e.event.task().unwrap_or_else(|| panic!("task"));
            let old = before.task(id).map_or(0, |x| x.life.failures());
            let new = after.task(id).map_or(0, |x| x.life.failures());
            assert_eq!(new, old + 1, "I5: the failure was not counted");
        }
    }
}

/// Runs one generated case. Panics on a broken invariant. Records the paths it reached.
fn simulate(specs: Vec<TaskSpec>, ops: &[Op], seen: &mut Seen) {
    let reg = Registry::parse(DEFAULT_REGISTRY).unwrap_or_else(|e| panic!("{e}"));
    let mut ledger = Ledger::new();
    let mut now = 1_000u64;
    let ctx = Ctx {
        now,
        registry: &reg,
    };
    let mut log = ledger
        .execute(Command::Plan(specs), &ctx)
        .unwrap_or_else(|e| panic!("{e}"));
    let mut max_token = 0u64;
    for op in ops {
        if let Op::Wait(dt) = op {
            now += dt;
            continue;
        }
        let Some(cmd) = command(&ledger, op) else {
            continue;
        };
        let before = ledger.clone();
        match ledger.execute(
            cmd,
            &Ctx {
                now,
                registry: &reg,
            },
        ) {
            Ok(events) => {
                check_step(&before, &ledger, &events, now, &mut max_token, &reg);
                count(&before, &ledger, &events, seen);
                log.extend(events);
            }
            Err(e) => {
                // Atomic: a rejected command changes nothing.
                assert_eq!(before, ledger);
                seen.wip_limits += usize::from(matches!(e, MeshError::WipLimit { .. }));
            }
        }
    }
    // Replay from JSONL text gives the same ledger and no issues.
    let text: String = log
        .iter()
        .map(|e| e.to_line().unwrap_or_else(|err| panic!("{err}")) + "\n")
        .collect();
    let replayed = store::replay(&text);
    assert!(replayed.issues.is_empty(), "{:?}", replayed.issues);
    assert_eq!(replayed.ledger, ledger);
}

fn count(before: &Ledger, after: &Ledger, events: &[Envelope], seen: &mut Seen) {
    for e in events {
        let id = e.event.task();
        let has_deps = id
            .and_then(|i| before.task(i))
            .is_some_and(|t| !t.spec.deps.is_empty());
        match &e.event {
            Event::Claimed { .. } if has_deps => seen.dependent_claims += 1,
            Event::Retried { .. } => seen.retries += 1,
            Event::Answered { .. } => seen.answers += 1,
            Event::LeaseExpired { .. } => seen.expiries += 1,
            Event::Completed { .. } => seen.completions += 1,
            _ => {}
        }
        let used = id.and_then(|i| after.task(i)).is_some_and(|t| {
            t.life.state() == State::Failed && t.life.failures() == t.life.max_failures()
        });
        seen.budget_used += usize::from(
            used && matches!(e.event, Event::Failed { .. } | Event::LeaseExpired { .. }),
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn invariants_hold_for_random_sequences(specs in tasks(), ops in prop::collection::vec(op(), 1..80)) {
        simulate(specs, &ops, &mut Seen::default());
    }

    /// I7: `plan::order` accepts exactly the acyclic plans, in dependency order.
    #[test]
    fn plans_are_rejected_exactly_when_they_have_a_cycle(
        edges in prop::collection::vec(prop::collection::vec(0..6usize, 0..3), 2..7)
    ) {
        let n = edges.len();
        let specs: Vec<TaskSpec> = edges
            .iter()
            .enumerate()
            .map(|(i, deps)| {
                let mut s = TaskSpec::new(tid(i), "T", Skill::new("build").unwrap_or_else(|e| panic!("{e}")));
                s.deps = deps.iter().filter(|d| **d < n).map(|d| tid(*d)).collect::<BTreeSet<_>>().into_iter().collect();
                s
            })
            .collect();
        let deps: BTreeMap<TaskId, Vec<TaskId>> = specs.iter().map(|s| (s.id.clone(), s.deps.clone())).collect();
        match plan::order(&Ledger::new(), specs) {
            Ok(ordered) => {
                let pos: BTreeMap<&TaskId, usize> = ordered.iter().enumerate().map(|(i, s)| (&s.id, i)).collect();
                prop_assert!(!has_cycle(&deps));
                for s in &ordered {
                    for d in &s.deps {
                        prop_assert!(pos[d] < pos[&s.id], "dependency after its task");
                    }
                }
            }
            Err(MeshError::Cycle(path)) => {
                prop_assert!(has_cycle(&deps));
                prop_assert_eq!(path.first(), path.last());
                for pair in path.windows(2) {
                    prop_assert!(deps[&pair[0]].contains(&pair[1]), "not an edge: {pair:?}");
                }
            }
            Err(e) => prop_assert!(false, "unexpected error {e}"),
        }
    }

    /// A broken line in the middle does not change the rest of the replay.
    #[test]
    fn replay_skips_a_garbage_line(garbage in "[ -~]{1,40}") {
        prop_assume!(!garbage.trim().is_empty() && serde_json::from_str::<Envelope>(&garbage).is_err());
        let add = |seq: u64, id: &str| Envelope {
            seq,
            ts: 1,
            actor: "coordinator".into(),
            event: Event::TaskAdded { task: TaskSpec::new(TaskId::new(id).unwrap_or_else(|e| panic!("{e}")), "T", Skill::new("build").unwrap_or_else(|e| panic!("{e}"))) },
        };
        let a = add(1, "a").to_line().unwrap_or_else(|e| panic!("{e}"));
        let b = add(2, "b").to_line().unwrap_or_else(|e| panic!("{e}"));
        let clean = store::replay(&format!("{a}\n{b}\n"));
        let dirty = store::replay(&format!("{a}\n{garbage}\n{b}\n"));
        prop_assert_eq!(&dirty.ledger, &clean.ledger);
        prop_assert_eq!(dirty.issues.len(), 1);
    }
}

fn has_cycle(deps: &BTreeMap<TaskId, Vec<TaskId>>) -> bool {
    fn visit(
        n: &TaskId,
        deps: &BTreeMap<TaskId, Vec<TaskId>>,
        color: &mut BTreeMap<TaskId, u8>,
    ) -> bool {
        match color.get(n) {
            Some(1) => return true,
            Some(2) => return false,
            _ => {}
        }
        color.insert(n.clone(), 1);
        let cyclic = deps[n].iter().any(|d| visit(d, deps, color));
        color.insert(n.clone(), 2);
        cyclic
    }
    let mut color = BTreeMap::new();
    deps.keys().any(|n| visit(n, deps, &mut color))
}

/// The generator reaches each important path. A fixed seed makes this test stable.
#[test]
fn generator_reaches_all_paths() {
    let seen = RefCell::new(Seen::default());
    let mut runner = TestRunner::new_with_rng(
        Config {
            cases: 256,
            ..Config::default()
        },
        TestRng::deterministic_rng(RngAlgorithm::ChaCha),
    );
    let strategy = (tasks(), prop::collection::vec(op(), 1..80));
    runner
        .run(&strategy, |(specs, ops)| {
            simulate(specs, &ops, &mut seen.borrow_mut());
            Ok(())
        })
        .unwrap_or_else(|e| panic!("{e}"));
    let seen = seen.into_inner();
    for (name, n) in [
        ("dependent claims", seen.dependent_claims),
        ("retries", seen.retries),
        ("answers", seen.answers),
        ("lease expiries", seen.expiries),
        ("used budgets", seen.budget_used),
        ("WIP limits", seen.wip_limits),
        ("completions", seen.completions),
    ] {
        assert!(
            n >= 5,
            "the generator reached {name} only {n} times: {seen:?}"
        );
    }
}

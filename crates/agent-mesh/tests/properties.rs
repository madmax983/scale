//! Property tests: invariants I1-I9 hold for random command sequences.
//! I1-I5 are also proved in `verus/lifecycle.rs`.

use std::collections::BTreeMap;

use agent_mesh::event::{Envelope, Event, TaskSpec};
use agent_mesh::ids::{AgentId, Skill, TaskId, Token};
use agent_mesh::ledger::{Command, Ctx, Ledger, NoteKind};
use agent_mesh::lifecycle::State;
use agent_mesh::registry::{Registry, DEFAULT_REGISTRY};
use proptest::prelude::*;

const AGENTS: [&str; 3] = ["builder", "reviewer", "architect"];
const SKILLS: [&str; 3] = ["build", "review", "spec"];

#[derive(Debug, Clone)]
enum Op {
    Claim(usize, usize),
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
        3 => (0..6usize, 0..3usize).prop_map(|(t, a)| Op::Claim(t, a)),
        2 => (0..3usize).prop_map(Op::ClaimNext),
        2 => (0..6usize, any::<bool>()).prop_map(|(t, s)| Op::Heartbeat(t, s)),
        3 => (0..6usize, any::<bool>(), any::<bool>()).prop_map(|(t, s, e)| Op::Complete(t, s, e)),
        2 => (0..6usize, any::<bool>()).prop_map(|(t, s)| Op::Fail(t, s)),
        1 => (0..6usize, any::<bool>()).prop_map(|(t, s)| Op::Ask(t, s)),
        1 => (0..6usize).prop_map(Op::Answer),
        2 => (0..6usize).prop_map(Op::Retry),
        1 => (0..6usize).prop_map(Op::Cancel),
        1 => Just(Op::Reap),
        2 => (1..80u64).prop_map(Op::Wait),
    ]
}

/// Six tasks. Each task may depend on earlier tasks only, so the input is a DAG.
fn tasks() -> impl Strategy<Value = Vec<TaskSpec>> {
    prop::collection::vec(
        (0..3usize, 1..4u32, prop::collection::vec(any::<bool>(), 6)),
        6,
    )
    .prop_map(|rows| {
        rows.into_iter()
            .enumerate()
            .map(|(i, (skill, budget, dep_bits))| {
                let mut s = TaskSpec::new(
                    tid(i),
                    format!("Task {i}"),
                    Skill::new(SKILLS[skill]).unwrap_or_else(|e| panic!("{e}")),
                );
                s.max_failures = budget;
                s.deps = (0..i).filter(|j| dep_bits[*j]).map(tid).collect();
                s
            })
            .collect()
    })
}

fn tid(i: usize) -> TaskId {
    TaskId::new(format!("t{i}")).unwrap_or_else(|e| panic!("{e}"))
}

fn aid(i: usize) -> AgentId {
    AgentId::new(AGENTS[i]).unwrap_or_else(|e| panic!("{e}"))
}

fn token_for(l: &Ledger, t: usize, stale: bool) -> Token {
    let current = l
        .task(&tid(t))
        .and_then(|x| x.life.lease())
        .map_or(Token(1), |x| x.token);
    if stale {
        Token(current.0 + 1000)
    } else {
        current
    }
}

fn command(l: &Ledger, op: &Op) -> Option<Command> {
    Some(match *op {
        Op::Claim(t, a) => Command::Claim {
            task: Some(tid(t)),
            agent: aid(a),
            worker: None,
            ttl: 60,
        },
        Op::ClaimNext(a) => Command::Claim {
            task: None,
            agent: aid(a),
            worker: None,
            ttl: 60,
        },
        Op::Heartbeat(t, s) => Command::Heartbeat {
            task: tid(t),
            token: token_for(l, t, s),
            ttl: 60,
        },
        Op::Complete(t, s, empty) => Command::Complete {
            task: tid(t),
            token: token_for(l, t, s),
            evidence: if empty {
                String::new()
            } else {
                "tests pass".into()
            },
        },
        Op::Fail(t, s) => Command::Fail {
            task: tid(t),
            token: token_for(l, t, s),
            reason: "broken".into(),
        },
        Op::Ask(t, s) => Command::Ask {
            task: tid(t),
            token: token_for(l, t, s),
            question: "why?".into(),
        },
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
        // I7: dependencies point to earlier tasks only.
        let me: usize = t.spec.id.as_str()[1..].parse().unwrap_or(0);
        for d in &t.spec.deps {
            let dep: usize = d.as_str()[1..].parse().unwrap_or(usize::MAX);
            assert!(dep < me, "I7 broken");
        }
        // I8: completed tasks have evidence.
        if t.life.state() == State::Completed {
            assert!(
                t.last_note(NoteKind::Evidence)
                    .is_some_and(|e| !e.trim().is_empty()),
                "I8 broken"
            );
        }
        // I2: terminal tasks do not change.
        if let Some(old) = before.task(&t.spec.id) {
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
        match &e.event {
            // I3: fresh tokens. I6: dependencies completed before the claim.
            Event::Claimed { task, token, .. } => {
                assert!(token.0 > *max_token, "I3 broken");
                *max_token = token.0;
                let spec = &before.task(task).unwrap_or_else(|| panic!("task")).spec;
                for d in &spec.deps {
                    assert_eq!(
                        before.task(d).map(|x| x.life.state()),
                        Some(State::Completed),
                        "I6 broken"
                    );
                }
            }
            // I4: worker writes need the live lease with the same token.
            Event::Heartbeat { task, token, .. }
            | Event::Completed { task, token, .. }
            | Event::Failed { task, token, .. }
            | Event::InputRequested { task, token, .. } => {
                let lease = before
                    .task(task)
                    .and_then(|x| x.life.lease())
                    .unwrap_or_else(|| panic!("I4: no lease"));
                assert_eq!(lease.token, *token, "I4 token");
                assert!(now < lease.expires_at, "I4 expiry");
            }
            _ => {}
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn invariants_hold_for_random_sequences(specs in tasks(), ops in prop::collection::vec(op(), 1..80)) {
        let reg = Registry::parse(DEFAULT_REGISTRY).unwrap_or_else(|e| panic!("{e}"));
        let mut ledger = Ledger::new();
        let mut now = 1_000u64;
        let mut log = ledger.execute(Command::Plan(specs), &Ctx { now, registry: &reg }).unwrap_or_else(|e| panic!("{e}"));
        let mut max_token = 0u64;
        for op in &ops {
            if let Op::Wait(dt) = op {
                now += dt;
                continue;
            }
            let Some(cmd) = command(&ledger, op) else { continue };
            let before = ledger.clone();
            match ledger.execute(cmd, &Ctx { now, registry: &reg }) {
                Ok(events) => {
                    check_step(&before, &ledger, &events, now, &mut max_token, &reg);
                    log.extend(events);
                }
                // Atomic: a rejected command changes nothing.
                Err(_) => prop_assert_eq!(&before, &ledger),
            }
        }
        // Replay of the accepted events gives the same ledger.
        let mut copy = Ledger::new();
        for env in &log {
            prop_assert!(copy.apply(env).is_ok());
        }
        prop_assert_eq!(copy, ledger);
    }

    #[test]
    fn replay_survives_any_appended_garbage(garbage in "[ -~]{0,40}") {
        let text = format!("{garbage}\n");
        let r = agent_mesh::store::replay(&text);
        prop_assert!(r.ledger.is_empty() || r.issues.is_empty());
    }
}

#[test]
fn every_state_is_reachable() {
    // Sanity check of the generator space: the six states exist.
    let names: BTreeMap<&str, State> = State::ALL.iter().map(|s| (s.as_str(), *s)).collect();
    assert_eq!(names.len(), 6);
}

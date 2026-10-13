//! Verus model of the agent mesh task lifecycle.
//!
//! This file is a model, not runtime code. `src/lifecycle.rs` follows it.
//! Run: `verus crates/agent-mesh/verus/lifecycle.rs`.
//!
//! Proved invariants (see docs/plans/2026-10-13-agent-mesh-design.md):
//! - I1: a task has a lease if and only if it is `Working`.
//! - I2: `Completed` and `Canceled` accept no transition.
//! - I3: each claim gets a token larger than every earlier token.
//! - I4: worker operations need the current token and a live lease.
//! - I5: `failures <= max_failures`; `Failed` exits only by retry or cancel.
use vstd::prelude::*;

verus! {

/// Task states. The names follow A2A 1.0.
pub enum State {
    Submitted,
    Working,
    InputRequired,
    Completed,
    Failed,
    Canceled,
}

/// A claim on a task.
pub struct Lease {
    pub token: nat,
    pub expires_at: nat,
}

/// One task.
pub struct Task {
    pub state: State,
    pub lease: Option<Lease>,
    pub failures: nat,
    pub max_failures: nat,
}

/// One task and the global token counter of the ledger.
pub struct Mesh {
    pub task: Task,
    pub next_token: nat,
}

/// Operations. Worker operations carry a token and the current time.
pub enum Op {
    Claim { now: nat, ttl: nat },
    Heartbeat { token: nat, now: nat, ttl: nat },
    Complete { token: nat, now: nat },
    Fail { token: nat, now: nat },
    Ask { token: nat, now: nat },
    Answer,
    Retry,
    Cancel,
    Expire { now: nat },
}

pub open spec fn is_terminal(s: State) -> bool {
    s is Completed || s is Canceled
}

/// Active states must keep budget for one more failure.
pub open spec fn is_active(s: State) -> bool {
    s is Submitted || s is Working || s is InputRequired
}

pub open spec fn inv(m: Mesh) -> bool {
    // I1
    &&& (m.task.state is Working) == (m.task.lease is Some)
    // I3 support: every issued token is below the counter.
    &&& (m.task.lease is Some ==> m.task.lease->0.token < m.next_token)
    // I5
    &&& m.task.failures <= m.task.max_failures
    &&& (is_active(m.task.state) ==> m.task.failures < m.task.max_failures)
    &&& m.task.max_failures >= 1
}

/// The caller holds the live lease with this token.
pub open spec fn holds(m: Mesh, token: nat, now: nat) -> bool {
    &&& m.task.state is Working
    &&& m.task.lease is Some
    &&& m.task.lease->0.token == token
    &&& now < m.task.lease->0.expires_at
}

pub open spec fn with_task(m: Mesh, t: Task) -> Mesh {
    Mesh { task: t, next_token: m.next_token }
}

pub open spec fn set_state(t: Task, s: State) -> Task {
    Task { state: s, lease: None, failures: t.failures, max_failures: t.max_failures }
}

/// The transition relation. `None` means the mesh rejects the operation.
pub open spec fn step(m: Mesh, op: Op) -> Option<Mesh> {
    let t = m.task;
    match op {
        Op::Claim { now, ttl } => if t.state is Submitted && ttl > 0 {
            Some(Mesh {
                task: Task {
                    state: State::Working,
                    lease: Some(Lease { token: m.next_token, expires_at: now + ttl }),
                    failures: t.failures,
                    max_failures: t.max_failures,
                },
                next_token: m.next_token + 1,
            })
        } else {
            None
        },
        Op::Heartbeat { token, now, ttl } => if holds(m, token, now) && ttl > 0 {
            Some(with_task(m, Task {
                state: State::Working,
                lease: Some(Lease { token, expires_at: now + ttl }),
                failures: t.failures,
                max_failures: t.max_failures,
            }))
        } else {
            None
        },
        Op::Complete { token, now } => if holds(m, token, now) {
            Some(with_task(m, set_state(t, State::Completed)))
        } else {
            None
        },
        Op::Fail { token, now } => if holds(m, token, now) {
            Some(with_task(m, Task {
                state: State::Failed,
                lease: None,
                failures: t.failures + 1,
                max_failures: t.max_failures,
            }))
        } else {
            None
        },
        Op::Ask { token, now } => if holds(m, token, now) {
            Some(with_task(m, set_state(t, State::InputRequired)))
        } else {
            None
        },
        Op::Answer => if t.state is InputRequired {
            Some(with_task(m, set_state(t, State::Submitted)))
        } else {
            None
        },
        Op::Retry => if t.state is Failed && t.failures < t.max_failures {
            Some(with_task(m, set_state(t, State::Submitted)))
        } else {
            None
        },
        Op::Cancel => if !is_terminal(t.state) {
            Some(with_task(m, set_state(t, State::Canceled)))
        } else {
            None
        },
        Op::Expire { now } => if t.state is Working && t.lease is Some
            && now >= t.lease->0.expires_at {
            let failures = t.failures + 1;
            let next = if failures < t.max_failures { State::Submitted } else { State::Failed };
            Some(with_task(m, Task {
                state: next,
                lease: None,
                failures,
                max_failures: t.max_failures,
            }))
        } else {
            None
        },
    }
}

/// Apply a sequence of operations. Stop at the first rejection.
pub open spec fn run(m: Mesh, ops: Seq<Op>) -> Option<Mesh>
    decreases ops.len(),
{
    if ops.len() == 0 {
        Some(m)
    } else {
        match step(m, ops[0]) {
            Some(m2) => run(m2, ops.subrange(1, ops.len() as int)),
            None => None,
        }
    }
}

pub open spec fn initial(max_failures: nat, next_token: nat) -> Mesh {
    Mesh {
        task: Task { state: State::Submitted, lease: None, failures: 0, max_failures },
        next_token,
    }
}

/// A new task satisfies the invariant.
pub proof fn lemma_initial_inv(max_failures: nat, next_token: nat)
    requires
        max_failures >= 1,
    ensures
        inv(initial(max_failures, next_token)),
{
}

/// I1, I3 support, I5: each accepted step keeps the invariant.
pub proof fn lemma_step_preserves_inv(m: Mesh, op: Op)
    requires
        inv(m),
    ensures
        step(m, op) matches Some(m2) ==> inv(m2),
{
}

/// The invariant holds after any accepted run.
pub proof fn lemma_run_preserves_inv(m: Mesh, ops: Seq<Op>)
    requires
        inv(m),
    ensures
        run(m, ops) matches Some(m2) ==> inv(m2),
    decreases ops.len(),
{
    if ops.len() > 0 {
        lemma_step_preserves_inv(m, ops[0]);
        if let Some(m1) = step(m, ops[0]) {
            lemma_run_preserves_inv(m1, ops.subrange(1, ops.len() as int));
        }
    }
}

/// I2: terminal states reject every operation.
pub proof fn lemma_terminal_absorbing(m: Mesh, op: Op)
    requires
        is_terminal(m.task.state),
    ensures
        step(m, op) is None,
{
}

/// I3: the token counter never goes down.
pub proof fn lemma_step_token_monotone(m: Mesh, op: Op)
    ensures
        step(m, op) matches Some(m2) ==> m2.next_token >= m.next_token,
{
}

/// I3: a claim issues the counter value and then increments the counter.
pub proof fn lemma_claim_token_fresh(m: Mesh, now: nat, ttl: nat)
    ensures
        step(m, Op::Claim { now, ttl }) matches Some(m2) ==> {
            &&& m2.task.lease->0.token == m.next_token
            &&& m2.next_token == m.next_token + 1
        },
{
}

/// I3: over a run, the counter never goes down.
pub proof fn lemma_run_token_monotone(m: Mesh, ops: Seq<Op>)
    ensures
        run(m, ops) matches Some(m2) ==> m2.next_token >= m.next_token,
    decreases ops.len(),
{
    if ops.len() > 0 {
        lemma_step_token_monotone(m, ops[0]);
        if let Some(m1) = step(m, ops[0]) {
            lemma_run_token_monotone(m1, ops.subrange(1, ops.len() as int));
        }
    }
}

/// I3: a claim after any run gets a token larger than a token issued before that run.
pub proof fn lemma_later_claim_larger(m: Mesh, ops: Seq<Op>, now: nat, ttl: nat)
    requires
        m.task.lease is Some,
        inv(m),
    ensures
        run(m, ops) matches Some(m2) ==> (step(m2, Op::Claim { now, ttl }) matches Some(m3)
            ==> m3.task.lease->0.token > m.task.lease->0.token),
{
    lemma_run_token_monotone(m, ops);
}

/// I4: an accepted worker operation proves the caller held the live lease.
pub proof fn lemma_fencing(m: Mesh, op: Op)
    ensures
        step(m, op) is Some ==> match op {
            Op::Heartbeat { token, now, .. } => holds(m, token, now),
            Op::Complete { token, now } => holds(m, token, now),
            Op::Fail { token, now } => holds(m, token, now),
            Op::Ask { token, now } => holds(m, token, now),
            _ => true,
        },
{
}

/// I4: a stale token is rejected.
pub proof fn lemma_stale_token_rejected(m: Mesh, token: nat, now: nat)
    requires
        m.task.lease is Some,
        m.task.lease->0.token != token,
    ensures
        step(m, Op::Complete { token, now }) is None,
        step(m, Op::Fail { token, now }) is None,
        step(m, Op::Ask { token, now }) is None,
        forall|ttl: nat| #[trigger] step(m, Op::Heartbeat { token, now, ttl }) is None,
{
}

/// I5: `Failed` exits only by a retry inside the budget, or by cancel.
pub proof fn lemma_failed_exits(m: Mesh, op: Op)
    requires
        m.task.state is Failed,
    ensures
        step(m, op) matches Some(m2) ==> {
            ||| (op is Retry && m.task.failures < m.task.max_failures
                && m2.task.state is Submitted)
            ||| (op is Cancel && m2.task.state is Canceled)
        },
{
}

/// I5: when the budget is used, a failed task can only be canceled.
pub proof fn lemma_budget_exhausted(m: Mesh, op: Op)
    requires
        inv(m),
        m.task.state is Failed,
        m.task.failures == m.task.max_failures,
    ensures
        step(m, op) matches Some(m2) ==> m2.task.state is Canceled,
{
}

fn main() {
}

} // verus!

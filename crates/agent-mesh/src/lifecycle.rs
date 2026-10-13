//! Task lifecycle: states, leases, and the transition rules.
//!
//! This module follows the Verus model in `verus/lifecycle.rs`.
//! Keep both in step. Invariants I1-I5 are proved there and tested here.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ids::{AgentId, Token};

/// Task states. The names follow A2A 1.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    /// Waiting for a claim.
    Submitted,
    /// Claimed. A lease exists.
    Working,
    /// Waiting for an answer from the coordinator.
    InputRequired,
    /// Done, with evidence. Terminal.
    Completed,
    /// Failed. Only the coordinator can retry it.
    Failed,
    /// Stopped by the coordinator. Terminal.
    Canceled,
}

impl State {
    /// All states, in board order.
    pub const ALL: [Self; 6] = [
        Self::Submitted,
        Self::Working,
        Self::InputRequired,
        Self::Failed,
        Self::Completed,
        Self::Canceled,
    ];

    /// `true` for states that accept no transition.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Canceled)
    }

    /// `true` for states that must keep budget for one more failure.
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Submitted | Self::Working | Self::InputRequired)
    }

    /// The A2A name of the state.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::Working => "working",
            Self::InputRequired => "input-required",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A claim on a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    /// The fencing token of the claim.
    pub token: Token,
    /// The role that holds the claim.
    pub agent: AgentId,
    /// A label for the worker instance.
    pub worker: String,
    /// Claim time (Unix seconds).
    pub claimed_at: u64,
    /// Expiry time (Unix seconds). The lease is live while `now < expires_at`.
    pub expires_at: u64,
}

/// Why the lifecycle rejected an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    /// The operation is not valid in this state.
    WrongState(State),
    /// The token is not the token of the current lease.
    StaleToken,
    /// The lease time is over.
    LeaseExpired,
    /// The failure budget is used.
    BudgetExhausted,
    /// The new expiry time is not after the current time.
    InvalidLease,
    /// The lease is still live, so it cannot expire.
    LeaseLive,
}

/// The lifecycle of one task. Only the methods below change it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Lifecycle {
    state: State,
    lease: Option<Lease>,
    failures: u32,
    max_failures: u32,
    attempts: u32,
}

impl Lifecycle {
    /// A new task in state `submitted`.
    #[must_use]
    pub const fn new(max_failures: u32) -> Self {
        Self {
            state: State::Submitted,
            lease: None,
            failures: 0,
            max_failures,
            attempts: 0,
        }
    }

    /// The current state.
    #[must_use]
    pub const fn state(&self) -> State {
        self.state
    }

    /// The current lease. It exists only in state `working` (I1).
    #[must_use]
    pub const fn lease(&self) -> Option<&Lease> {
        self.lease.as_ref()
    }

    /// The number of failures (fail and lease expiry).
    #[must_use]
    pub const fn failures(&self) -> u32 {
        self.failures
    }

    /// The failure budget.
    #[must_use]
    pub const fn max_failures(&self) -> u32 {
        self.max_failures
    }

    /// The number of claims.
    #[must_use]
    pub const fn attempts(&self) -> u32 {
        self.attempts
    }

    /// Checks that `token` holds the live lease at `now` (I4).
    ///
    /// # Errors
    /// A [`Rejection`] if the task is not working, the token is stale, or the lease expired.
    pub fn holds(&self, token: Token, now: u64) -> Result<&Lease, Rejection> {
        match (self.state, &self.lease) {
            (State::Working, Some(lease)) if lease.token != token => Err(Rejection::StaleToken),
            (State::Working, Some(lease)) if now >= lease.expires_at => {
                Err(Rejection::LeaseExpired)
            }
            (State::Working, Some(lease)) => Ok(lease),
            (state, _) => Err(Rejection::WrongState(state)),
        }
    }

    /// `submitted` -> `working`. The ledger gives a fresh token (I3).
    ///
    /// # Errors
    /// A [`Rejection`] if the task is not submitted or the lease has no time.
    pub fn claim(&mut self, lease: Lease) -> Result<(), Rejection> {
        if self.state != State::Submitted {
            return Err(Rejection::WrongState(self.state));
        }
        if lease.expires_at <= lease.claimed_at {
            return Err(Rejection::InvalidLease);
        }
        self.state = State::Working;
        self.lease = Some(lease);
        self.attempts = self.attempts.saturating_add(1);
        Ok(())
    }

    /// Extends the live lease to `expires_at`.
    ///
    /// # Errors
    /// A [`Rejection`] if the caller does not hold the live lease.
    pub fn heartbeat(&mut self, token: Token, now: u64, expires_at: u64) -> Result<(), Rejection> {
        self.holds(token, now)?;
        if expires_at <= now {
            return Err(Rejection::InvalidLease);
        }
        if let Some(lease) = self.lease.as_mut() {
            lease.expires_at = expires_at;
        }
        Ok(())
    }

    /// `working` -> `completed`.
    ///
    /// # Errors
    /// A [`Rejection`] if the caller does not hold the live lease.
    pub fn complete(&mut self, token: Token, now: u64) -> Result<(), Rejection> {
        self.holds(token, now)?;
        self.finish(State::Completed);
        Ok(())
    }

    /// `working` -> `failed`. Counts one failure.
    ///
    /// # Errors
    /// A [`Rejection`] if the caller does not hold the live lease.
    pub fn fail(&mut self, token: Token, now: u64) -> Result<(), Rejection> {
        self.holds(token, now)?;
        self.failures = self.failures.saturating_add(1);
        self.finish(State::Failed);
        Ok(())
    }

    /// `working` -> `input-required`. Releases the lease.
    ///
    /// # Errors
    /// A [`Rejection`] if the caller does not hold the live lease.
    pub fn ask(&mut self, token: Token, now: u64) -> Result<(), Rejection> {
        self.holds(token, now)?;
        self.finish(State::InputRequired);
        Ok(())
    }

    /// `input-required` -> `submitted`.
    ///
    /// # Errors
    /// A [`Rejection`] if the task does not wait for input.
    pub fn answer(&mut self) -> Result<(), Rejection> {
        if self.state != State::InputRequired {
            return Err(Rejection::WrongState(self.state));
        }
        self.finish(State::Submitted);
        Ok(())
    }

    /// `failed` -> `submitted`, while budget is left (I5).
    ///
    /// # Errors
    /// A [`Rejection`] if the task is not failed or the budget is used.
    pub fn retry(&mut self) -> Result<(), Rejection> {
        if self.state != State::Failed {
            return Err(Rejection::WrongState(self.state));
        }
        if self.failures >= self.max_failures {
            return Err(Rejection::BudgetExhausted);
        }
        self.finish(State::Submitted);
        Ok(())
    }

    /// Any non-terminal state -> `canceled`.
    ///
    /// # Errors
    /// A [`Rejection`] if the task is terminal (I2).
    pub fn cancel(&mut self) -> Result<(), Rejection> {
        if self.state.is_terminal() {
            return Err(Rejection::WrongState(self.state));
        }
        self.finish(State::Canceled);
        Ok(())
    }

    /// Ends an overdue lease. Counts one failure. The task goes to `submitted`,
    /// or to `failed` when the budget is used.
    ///
    /// # Errors
    /// A [`Rejection`] if the task is not working or the lease is still live.
    pub fn expire(&mut self, now: u64) -> Result<(), Rejection> {
        match (self.state, &self.lease) {
            (State::Working, Some(lease)) if now < lease.expires_at => Err(Rejection::LeaseLive),
            (State::Working, Some(_)) => {
                self.failures = self.failures.saturating_add(1);
                let next = if self.failures < self.max_failures {
                    State::Submitted
                } else {
                    State::Failed
                };
                self.finish(next);
                Ok(())
            }
            (state, _) => Err(Rejection::WrongState(state)),
        }
    }

    /// `true` if invariants I1 and I5 hold.
    #[must_use]
    pub fn invariant_holds(&self) -> bool {
        (self.state == State::Working) == self.lease.is_some()
            && self.max_failures >= 1
            && self.failures <= self.max_failures
            && (!self.state.is_active() || self.failures < self.max_failures)
    }

    /// Moves to `state` and drops the lease.
    fn finish(&mut self, state: State) {
        self.state = state;
        self.lease = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent() -> AgentId {
        AgentId::new("builder").unwrap_or_else(|e| panic!("{e}"))
    }

    fn lease(token: u64, now: u64, ttl: u64) -> Lease {
        Lease {
            token: Token(token),
            agent: agent(),
            worker: "w".into(),
            claimed_at: now,
            expires_at: now + ttl,
        }
    }

    fn working(token: u64) -> Lifecycle {
        let mut l = Lifecycle::new(2);
        assert_eq!(l.claim(lease(token, 100, 50)), Ok(()));
        l
    }

    #[test]
    fn terminal_and_active_states() {
        assert!(State::Completed.is_terminal());
        assert!(State::Canceled.is_terminal());
        assert!(!State::Failed.is_terminal());
        assert!(State::Submitted.is_active());
        assert!(State::Working.is_active());
        assert!(State::InputRequired.is_active());
        assert!(!State::Failed.is_active());
        assert_eq!(State::InputRequired.to_string(), "input-required");
    }

    #[test]
    fn claim_sets_lease_and_counts_attempt() {
        let l = working(1);
        assert_eq!(l.state(), State::Working);
        assert_eq!(l.attempts(), 1);
        assert_eq!(l.lease().map(|x| x.token), Some(Token(1)));
        assert!(l.invariant_holds());
    }

    #[test]
    fn claim_needs_submitted_and_time() {
        let mut l = working(1);
        assert_eq!(
            l.claim(lease(2, 100, 50)),
            Err(Rejection::WrongState(State::Working))
        );
        let mut fresh = Lifecycle::new(1);
        assert_eq!(fresh.claim(lease(1, 100, 0)), Err(Rejection::InvalidLease));
    }

    #[test]
    fn heartbeat_extends_only_with_current_token() {
        let mut l = working(3);
        assert_eq!(l.heartbeat(Token(3), 120, 500), Ok(()));
        assert_eq!(l.lease().map(|x| x.expires_at), Some(500));
        assert_eq!(l.heartbeat(Token(2), 130, 600), Err(Rejection::StaleToken));
        assert_eq!(
            l.heartbeat(Token(3), 130, 130),
            Err(Rejection::InvalidLease)
        );
    }

    #[test]
    fn worker_ops_reject_stale_token_and_expired_lease() {
        let mut l = working(3);
        assert_eq!(l.complete(Token(4), 110), Err(Rejection::StaleToken));
        assert_eq!(l.fail(Token(2), 110), Err(Rejection::StaleToken));
        assert_eq!(l.ask(Token(9), 110), Err(Rejection::StaleToken));
        assert_eq!(l.complete(Token(3), 150), Err(Rejection::LeaseExpired));
        assert_eq!(l.state(), State::Working);
    }

    #[test]
    fn complete_clears_lease() {
        let mut l = working(1);
        assert_eq!(l.complete(Token(1), 120), Ok(()));
        assert_eq!(l.state(), State::Completed);
        assert!(l.lease().is_none());
    }

    #[test]
    fn terminal_states_reject_everything() {
        let mut done = working(1);
        assert_eq!(done.complete(Token(1), 120), Ok(()));
        let mut canceled = Lifecycle::new(1);
        assert_eq!(canceled.cancel(), Ok(()));
        for l in [&mut done, &mut canceled] {
            let s = l.state();
            assert_eq!(l.claim(lease(5, 200, 10)), Err(Rejection::WrongState(s)));
            assert_eq!(l.answer(), Err(Rejection::WrongState(s)));
            assert_eq!(l.retry(), Err(Rejection::WrongState(s)));
            assert_eq!(l.cancel(), Err(Rejection::WrongState(s)));
            assert_eq!(l.expire(10_000), Err(Rejection::WrongState(s)));
            assert!(l.complete(Token(1), 120).is_err());
        }
    }

    #[test]
    fn fail_then_retry_within_budget() {
        let mut l = working(1);
        assert_eq!(l.fail(Token(1), 110), Ok(()));
        assert_eq!(l.state(), State::Failed);
        assert_eq!(l.failures(), 1);
        assert_eq!(l.retry(), Ok(()));
        assert_eq!(l.state(), State::Submitted);
        assert_eq!(l.claim(lease(2, 120, 10)), Ok(()));
        assert_eq!(l.fail(Token(2), 125), Ok(()));
        assert_eq!(l.failures(), 2);
        assert_eq!(l.retry(), Err(Rejection::BudgetExhausted));
        assert_eq!(l.cancel(), Ok(()));
        assert!(l.invariant_holds());
    }

    #[test]
    fn ask_releases_lease_and_answer_resubmits() {
        let mut l = working(1);
        assert_eq!(l.ask(Token(1), 110), Ok(()));
        assert_eq!(l.state(), State::InputRequired);
        assert!(l.lease().is_none());
        assert_eq!(l.answer(), Ok(()));
        assert_eq!(l.state(), State::Submitted);
        assert_eq!(l.failures(), 0);
    }

    #[test]
    fn expire_resubmits_until_budget_is_used() {
        let mut l = working(1);
        assert_eq!(l.expire(149), Err(Rejection::LeaseLive));
        assert_eq!(l.expire(150), Ok(()));
        assert_eq!(l.state(), State::Submitted);
        assert_eq!(l.failures(), 1);
        assert_eq!(l.claim(lease(2, 200, 10)), Ok(()));
        assert_eq!(l.expire(210), Ok(()));
        assert_eq!(l.state(), State::Failed);
        assert_eq!(l.failures(), 2);
        assert_eq!(l.retry(), Err(Rejection::BudgetExhausted));
        assert!(l.invariant_holds());
    }

    #[test]
    fn cancel_from_each_non_terminal_state() {
        let mut submitted = Lifecycle::new(1);
        let mut work = working(1);
        let mut input = working(1);
        assert_eq!(input.ask(Token(1), 110), Ok(()));
        let mut failed = working(1);
        assert_eq!(failed.fail(Token(1), 110), Ok(()));
        for l in [&mut submitted, &mut work, &mut input, &mut failed] {
            assert_eq!(l.cancel(), Ok(()));
            assert_eq!(l.state(), State::Canceled);
            assert!(l.lease().is_none());
        }
    }
}

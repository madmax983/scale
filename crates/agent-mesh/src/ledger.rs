//! The ledger: task state rebuilt from events, and the commands that make events.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::error::{MeshError, Result};
use crate::event::{validate_text, Envelope, Event, TaskSpec, MAX_TEXT, MAX_WORKER};
use crate::ids::{AgentId, TaskId, Token};
use crate::lifecycle::{Lease, Lifecycle, Rejection, State};
use crate::plan;
use crate::registry::Registry;

/// The actor name of coordinator commands.
pub const COORDINATOR: &str = "coordinator";
/// The actor name of automatic events (lease expiry).
pub const SYSTEM: &str = "mesh";
/// Default lease time in seconds.
pub const DEFAULT_TTL: u64 = 1_800;
/// Longest lease time in seconds.
pub const MAX_TTL: u64 = 86_400;

/// The kind of a task note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteKind {
    /// Proof of completion.
    Evidence,
    /// Why a task failed.
    Reason,
    /// A worker question.
    Question,
    /// A coordinator answer.
    Answer,
    /// Why a task was canceled.
    Cancel,
    /// A lease expired.
    Expired,
}

/// A note on a task, from an event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Note {
    /// Event sequence number.
    pub seq: u64,
    /// Event time.
    pub ts: u64,
    /// Event actor.
    pub actor: String,
    /// Kind.
    pub kind: NoteKind,
    /// Text.
    pub text: String,
}

/// A task and its history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Task {
    /// The definition.
    pub spec: TaskSpec,
    /// The lifecycle.
    pub life: Lifecycle,
    /// Notes, oldest first.
    pub notes: Vec<Note>,
    /// Time of the `task_added` event.
    pub created_at: u64,
    /// Time of the last event.
    pub updated_at: u64,
}

impl Task {
    /// The text of the last note of this kind.
    #[must_use]
    pub fn last_note(&self, kind: NoteKind) -> Option<&str> {
        self.notes
            .iter()
            .rev()
            .find(|n| n.kind == kind)
            .map(|n| n.text.as_str())
    }
}

/// Whether a `submitted` task can start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "deps", rename_all = "snake_case")]
pub enum Readiness {
    /// All dependencies are completed.
    Ready,
    /// Some dependencies are not done yet.
    Waiting(Vec<TaskId>),
    /// Some dependencies failed or were canceled.
    Blocked(Vec<TaskId>),
    /// The task is not `submitted`.
    NotSubmitted,
}

/// A request to change the ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Add one task.
    Add(TaskSpec),
    /// Add many tasks in one atomic step.
    Plan(Vec<TaskSpec>),
    /// Claim a task. `task: None` takes the first ready task for the agent.
    Claim {
        /// The task.
        task: Option<TaskId>,
        /// The role.
        agent: AgentId,
        /// The worker label. Default: the role id.
        worker: Option<String>,
        /// Lease time in seconds.
        ttl: u64,
    },
    /// Extend a lease.
    Heartbeat {
        /// The task.
        task: TaskId,
        /// The lease token.
        token: Token,
        /// New lease time in seconds, from now.
        ttl: u64,
    },
    /// Complete a task.
    Complete {
        /// The task.
        task: TaskId,
        /// The lease token.
        token: Token,
        /// Proof.
        evidence: String,
    },
    /// Fail a task.
    Fail {
        /// The task.
        task: TaskId,
        /// The lease token.
        token: Token,
        /// Why.
        reason: String,
    },
    /// Ask the coordinator a question.
    Ask {
        /// The task.
        task: TaskId,
        /// The lease token.
        token: Token,
        /// The question.
        question: String,
    },
    /// Answer a question.
    Answer {
        /// The task.
        task: TaskId,
        /// The answer.
        text: String,
    },
    /// Retry a failed task.
    Retry {
        /// The task.
        task: TaskId,
    },
    /// Cancel a task.
    Cancel {
        /// The task.
        task: TaskId,
        /// Why.
        reason: String,
    },
    /// Expire all overdue leases.
    Reap,
}

/// The context of a command.
#[derive(Debug, Clone, Copy)]
pub struct Ctx<'a> {
    /// The current time (Unix seconds).
    pub now: u64,
    /// The registry.
    pub registry: &'a Registry,
}

/// Task state rebuilt from events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ledger {
    tasks: BTreeMap<TaskId, Task>,
    order: Vec<TaskId>,
    next_seq: u64,
    next_token: u64,
}

impl Default for Ledger {
    fn default() -> Self {
        Self::new()
    }
}

impl Ledger {
    /// An empty ledger.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            tasks: BTreeMap::new(),
            order: Vec::new(),
            next_seq: 1,
            next_token: 1,
        }
    }

    /// Records the sequence number and token of a parsed envelope, valid or not.
    /// This keeps numbers fresh after a merge that adds conflicting lines.
    pub fn observe(&mut self, env: &Envelope) {
        self.next_seq = self.next_seq.max(env.seq.saturating_add(1));
        if let Event::Claimed { token, .. } = &env.event {
            self.next_token = self.next_token.max(token.0.saturating_add(1));
        }
    }

    /// Applies one event. The ledger does not change if the event is not valid.
    ///
    /// # Errors
    /// The [`MeshError`] that tells why the event is not valid.
    pub fn apply(&mut self, env: &Envelope) -> Result<()> {
        let ts = env.ts;
        match &env.event {
            Event::TaskAdded { task } => self.add_task(task, ts)?,
            Event::Claimed {
                task,
                agent,
                worker,
                token,
                expires_at,
            } => {
                if token.0 < self.next_token {
                    return Err(MeshError::TokenReused {
                        task: task.clone(),
                        token: *token,
                    });
                }
                validate_text("worker", worker, MAX_WORKER, true, true)?;
                self.check_ready(task)?;
                let lease = Lease {
                    token: *token,
                    agent: agent.clone(),
                    worker: worker.clone(),
                    claimed_at: ts,
                    expires_at: *expires_at,
                };
                self.life_op(task, "claim", None, |l| l.claim(lease))?;
            }
            Event::Heartbeat {
                task,
                token,
                expires_at,
            } => self.life_op(task, "heartbeat", Some(*token), |l| {
                l.heartbeat(*token, ts, *expires_at)
            })?,
            Event::Completed {
                task,
                token,
                evidence,
            } => {
                validate_text("evidence", evidence, MAX_TEXT, true, false)?;
                self.life_op(task, "complete", Some(*token), |l| l.complete(*token, ts))?;
                self.note(task, env, NoteKind::Evidence, evidence);
            }
            Event::Failed {
                task,
                token,
                reason,
            } => {
                validate_text("reason", reason, MAX_TEXT, true, false)?;
                self.life_op(task, "fail", Some(*token), |l| l.fail(*token, ts))?;
                self.note(task, env, NoteKind::Reason, reason);
            }
            Event::InputRequested {
                task,
                token,
                question,
            } => {
                validate_text("question", question, MAX_TEXT, true, false)?;
                self.life_op(task, "ask", Some(*token), |l| l.ask(*token, ts))?;
                self.note(task, env, NoteKind::Question, question);
            }
            Event::Answered { task, answer } => {
                validate_text("answer", answer, MAX_TEXT, true, false)?;
                self.life_op(task, "answer", None, Lifecycle::answer)?;
                self.note(task, env, NoteKind::Answer, answer);
            }
            Event::Retried { task } => self.life_op(task, "retry", None, Lifecycle::retry)?,
            Event::Canceled { task, reason } => {
                validate_text("reason", reason, MAX_TEXT, true, false)?;
                self.life_op(task, "cancel", None, Lifecycle::cancel)?;
                self.note(task, env, NoteKind::Cancel, reason);
            }
            Event::LeaseExpired { task, token } => {
                let current = self.get(task)?.life.lease().map(|l| l.token);
                if current.is_some_and(|t| t != *token) {
                    return Err(MeshError::StaleToken {
                        task: task.clone(),
                        token: *token,
                    });
                }
                self.life_op(task, "expire", Some(*token), |l| l.expire(ts))?;
                let text = format!("lease with token {token} expired");
                self.note(task, env, NoteKind::Expired, &text);
            }
        }
        if let Some(t) = self.tasks.get_mut(env.event.task()) {
            t.updated_at = ts;
        }
        self.observe(env);
        Ok(())
    }

    fn get(&self, id: &TaskId) -> Result<&Task> {
        self.tasks
            .get(id)
            .ok_or_else(|| MeshError::UnknownTask(id.clone()))
    }

    fn add_task(&mut self, spec: &TaskSpec, ts: u64) -> Result<()> {
        spec.validate()?;
        if self.tasks.contains_key(&spec.id) {
            return Err(MeshError::DuplicateTask(spec.id.clone()));
        }
        if let Some(dep) = spec.deps.iter().find(|d| !self.tasks.contains_key(*d)) {
            return Err(MeshError::UnknownDependency {
                task: spec.id.clone(),
                dep: dep.clone(),
            });
        }
        let task = Task {
            spec: spec.clone(),
            life: Lifecycle::new(spec.max_failures),
            notes: Vec::new(),
            created_at: ts,
            updated_at: ts,
        };
        self.tasks.insert(spec.id.clone(), task);
        self.order.push(spec.id.clone());
        Ok(())
    }

    fn check_ready(&self, id: &TaskId) -> Result<()> {
        let task = self.get(id)?;
        match self.readiness(id) {
            Readiness::Ready => Ok(()),
            Readiness::Waiting(waiting) | Readiness::Blocked(waiting) => Err(MeshError::NotReady {
                task: id.clone(),
                waiting,
            }),
            Readiness::NotSubmitted => Err(MeshError::InvalidTransition {
                task: id.clone(),
                op: "claim",
                state: task.life.state(),
            }),
        }
    }

    /// Runs one lifecycle step and maps a rejection to a [`MeshError`].
    fn life_op(
        &mut self,
        id: &TaskId,
        op: &'static str,
        token: Option<Token>,
        step: impl FnOnce(&mut Lifecycle) -> std::result::Result<(), Rejection>,
    ) -> Result<()> {
        let task = self
            .tasks
            .get_mut(id)
            .ok_or_else(|| MeshError::UnknownTask(id.clone()))?;
        let max = task.life.max_failures();
        step(&mut task.life).map_err(|r| reject(id, op, token, max, r))
    }

    fn note(&mut self, id: &TaskId, env: &Envelope, kind: NoteKind, text: &str) {
        if let Some(task) = self.tasks.get_mut(id) {
            task.notes.push(Note {
                seq: env.seq,
                ts: env.ts,
                actor: env.actor.clone(),
                kind,
                text: text.to_string(),
            });
        }
    }

    /// Checks that `token` holds the live lease of the task at `now`. It changes nothing.
    ///
    /// # Errors
    /// [`MeshError::UnknownTask`], [`MeshError::InvalidTransition`],
    /// [`MeshError::StaleToken`], or [`MeshError::LeaseExpired`].
    pub fn check_fence(&self, id: &TaskId, token: Token, now: u64) -> Result<&Lease> {
        let task = self.get(id)?;
        task.life
            .holds(token, now)
            .map_err(|r| reject(id, "fence", Some(token), task.life.max_failures(), r))
    }

    /// Runs a command. It checks the command, makes the events, and applies them.
    /// The step is atomic: on error the ledger does not change.
    ///
    /// # Errors
    /// The [`MeshError`] that tells why the command is not valid.
    pub fn execute(&mut self, cmd: Command, ctx: &Ctx<'_>) -> Result<Vec<Envelope>> {
        let mut draft = self.clone();
        let mut out = Vec::new();
        draft.run(cmd, ctx, &mut out)?;
        *self = draft;
        Ok(out)
    }

    fn run(&mut self, cmd: Command, ctx: &Ctx<'_>, out: &mut Vec<Envelope>) -> Result<()> {
        let now = ctx.now;
        match cmd {
            Command::Add(spec) => {
                check_served(ctx.registry, &spec)?;
                self.emit(out, now, COORDINATOR, Event::TaskAdded { task: spec })
            }
            Command::Plan(specs) => {
                let ordered = plan::order(self, specs)?;
                ordered
                    .iter()
                    .try_for_each(|s| check_served(ctx.registry, s))?;
                ordered.into_iter().try_for_each(|task| {
                    self.emit(out, now, COORDINATOR, Event::TaskAdded { task })
                })
            }
            Command::Claim {
                task,
                agent,
                worker,
                ttl,
            } => self.claim(out, ctx, task, agent, worker, ttl),
            Command::Heartbeat { task, token, ttl } => {
                check_ttl(ttl)?;
                let actor = self.lease_actor(&task);
                let expires_at = now.saturating_add(ttl);
                self.emit(
                    out,
                    now,
                    &actor,
                    Event::Heartbeat {
                        task,
                        token,
                        expires_at,
                    },
                )
            }
            Command::Complete {
                task,
                token,
                evidence,
            } => {
                let actor = self.lease_actor(&task);
                self.emit(
                    out,
                    now,
                    &actor,
                    Event::Completed {
                        task,
                        token,
                        evidence,
                    },
                )
            }
            Command::Fail {
                task,
                token,
                reason,
            } => {
                let actor = self.lease_actor(&task);
                self.emit(
                    out,
                    now,
                    &actor,
                    Event::Failed {
                        task,
                        token,
                        reason,
                    },
                )
            }
            Command::Ask {
                task,
                token,
                question,
            } => {
                let actor = self.lease_actor(&task);
                self.emit(
                    out,
                    now,
                    &actor,
                    Event::InputRequested {
                        task,
                        token,
                        question,
                    },
                )
            }
            Command::Answer { task, text } => self.emit(
                out,
                now,
                COORDINATOR,
                Event::Answered { task, answer: text },
            ),
            Command::Retry { task } => self.emit(out, now, COORDINATOR, Event::Retried { task }),
            Command::Cancel { task, reason } => {
                self.emit(out, now, COORDINATOR, Event::Canceled { task, reason })
            }
            Command::Reap => {
                let overdue: Vec<(TaskId, Token)> = self
                    .overdue(now)
                    .iter()
                    .filter_map(|t| t.life.lease().map(|l| (t.spec.id.clone(), l.token)))
                    .collect();
                overdue.into_iter().try_for_each(|(task, token)| {
                    self.emit(out, now, SYSTEM, Event::LeaseExpired { task, token })
                })
            }
        }
    }

    fn claim(
        &mut self,
        out: &mut Vec<Envelope>,
        ctx: &Ctx<'_>,
        task: Option<TaskId>,
        agent: AgentId,
        worker: Option<String>,
        ttl: u64,
    ) -> Result<()> {
        check_ttl(ttl)?;
        let card = ctx
            .registry
            .get(&agent)
            .ok_or_else(|| MeshError::UnknownAgent(agent.clone()))?;
        let id = match task {
            Some(id) => id,
            None => self
                .ready_for(ctx.registry, Some(&agent))
                .first()
                .map(|t| t.spec.id.clone())
                .ok_or_else(|| MeshError::NothingReady(agent.clone()))?,
        };
        let skill = &self.get(&id)?.spec.skill;
        if !card.has_skill(skill) {
            return Err(MeshError::SkillMismatch {
                task: id,
                agent,
                skill: skill.clone(),
            });
        }
        self.check_ready(&id)?;
        let active = self.active_count(&agent);
        if active >= card.max_active as usize {
            return Err(MeshError::WipLimit {
                agent,
                active,
                limit: card.max_active,
            });
        }
        let worker = worker.unwrap_or_else(|| agent.to_string());
        let event = Event::Claimed {
            task: id,
            agent: agent.clone(),
            worker,
            token: self.next_token(),
            expires_at: ctx.now.saturating_add(ttl),
        };
        self.emit(out, ctx.now, agent.as_str(), event)
    }

    fn emit(&mut self, out: &mut Vec<Envelope>, now: u64, actor: &str, event: Event) -> Result<()> {
        let env = Envelope {
            seq: self.next_seq,
            ts: now,
            actor: actor.to_string(),
            event,
        };
        self.apply(&env)?;
        out.push(env);
        Ok(())
    }

    /// The role of the current lease, or the coordinator.
    fn lease_actor(&self, id: &TaskId) -> String {
        self.task(id)
            .and_then(|t| t.life.lease())
            .map_or_else(|| COORDINATOR.to_string(), |l| l.agent.to_string())
    }

    /// The task with this id.
    #[must_use]
    pub fn task(&self, id: &TaskId) -> Option<&Task> {
        self.tasks.get(id)
    }

    /// All tasks, in the order they were added.
    pub fn tasks(&self) -> impl Iterator<Item = &Task> {
        self.order.iter().filter_map(|id| self.tasks.get(id))
    }

    /// The number of tasks.
    #[must_use]
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// `true` if there is no task.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Whether the task can start. Derived from the dependency states.
    #[must_use]
    pub fn readiness(&self, id: &TaskId) -> Readiness {
        let Some(task) = self.tasks.get(id) else {
            return Readiness::NotSubmitted;
        };
        if task.life.state() != State::Submitted {
            return Readiness::NotSubmitted;
        }
        let mut waiting = Vec::new();
        let mut blocked = Vec::new();
        for dep in &task.spec.deps {
            match self.tasks.get(dep).map(|t| t.life.state()) {
                Some(State::Completed) => {}
                Some(State::Failed | State::Canceled) | None => blocked.push(dep.clone()),
                Some(_) => waiting.push(dep.clone()),
            }
        }
        if !blocked.is_empty() {
            Readiness::Blocked(blocked)
        } else if !waiting.is_empty() {
            Readiness::Waiting(waiting)
        } else {
            Readiness::Ready
        }
    }

    /// Ready tasks, in the order they were added. With `agent`, only tasks the agent can do.
    #[must_use]
    pub fn ready_for(&self, registry: &Registry, agent: Option<&AgentId>) -> Vec<&Task> {
        let card = agent.map(|a| registry.get(a));
        self.tasks()
            .filter(|t| self.readiness(&t.spec.id) == Readiness::Ready)
            .filter(|t| match card {
                None => true,
                Some(card) => card.is_some_and(|c| c.has_skill(&t.spec.skill)),
            })
            .collect()
    }

    /// The number of `working` tasks that the agent holds.
    #[must_use]
    pub fn active_count(&self, agent: &AgentId) -> usize {
        self.tasks()
            .filter(|t| t.life.lease().is_some_and(|l| &l.agent == agent))
            .count()
    }

    /// `working` tasks with a lease that is over at `now`.
    #[must_use]
    pub fn overdue(&self, now: u64) -> Vec<&Task> {
        self.tasks()
            .filter(|t| t.life.lease().is_some_and(|l| l.expires_at <= now))
            .collect()
    }

    /// The sequence number of the next event.
    #[must_use]
    pub const fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// The token of the next claim.
    #[must_use]
    pub const fn next_token(&self) -> Token {
        Token(self.next_token)
    }
}

fn check_ttl(ttl: u64) -> Result<()> {
    if (1..=MAX_TTL).contains(&ttl) {
        Ok(())
    } else {
        Err(MeshError::InvalidTtl { ttl, max: MAX_TTL })
    }
}

fn check_served(registry: &Registry, spec: &TaskSpec) -> Result<()> {
    if registry.serves(&spec.skill) {
        Ok(())
    } else {
        Err(MeshError::UnservedSkill {
            task: spec.id.clone(),
            skill: spec.skill.clone(),
        })
    }
}

fn reject(
    id: &TaskId,
    op: &'static str,
    token: Option<Token>,
    max: u32,
    r: Rejection,
) -> MeshError {
    let task = id.clone();
    match r {
        Rejection::WrongState(state) => MeshError::InvalidTransition { task, op, state },
        Rejection::StaleToken => MeshError::StaleToken {
            task,
            token: token.unwrap_or(Token(0)),
        },
        Rejection::LeaseExpired => MeshError::LeaseExpired(task),
        Rejection::BudgetExhausted => MeshError::BudgetExhausted { task, max },
        Rejection::InvalidLease => MeshError::Ledger(format!(
            "task `{task}`: the lease expiry must be after the event time"
        )),
        Rejection::LeaseLive => MeshError::Ledger(format!(
            "task `{task}`: the lease is still live, so it cannot expire"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Event;
    use crate::ids::Skill;
    use crate::lifecycle::State;
    use crate::registry::DEFAULT_REGISTRY;

    fn reg() -> Registry {
        Registry::parse(DEFAULT_REGISTRY).unwrap_or_else(|e| panic!("{e}"))
    }

    fn tid(s: &str) -> TaskId {
        TaskId::new(s).unwrap_or_else(|e| panic!("{e}"))
    }

    fn aid(s: &str) -> AgentId {
        AgentId::new(s).unwrap_or_else(|e| panic!("{e}"))
    }

    fn spec(id: &str, skill: &str, deps: &[&str]) -> TaskSpec {
        let mut s = TaskSpec::new(
            tid(id),
            format!("Task {id}"),
            Skill::new(skill).unwrap_or_else(|e| panic!("{e}")),
        );
        s.deps = deps.iter().map(|d| tid(d)).collect();
        s
    }

    fn run(l: &mut Ledger, r: &Registry, now: u64, cmd: Command) -> Result<Vec<Envelope>> {
        l.execute(cmd, &Ctx { now, registry: r })
    }

    fn ok(l: &mut Ledger, r: &Registry, now: u64, cmd: Command) -> Vec<Envelope> {
        run(l, r, now, cmd).unwrap_or_else(|e| panic!("{e}"))
    }

    fn claim(l: &mut Ledger, r: &Registry, now: u64, task: &str, agent: &str) -> Token {
        let evs = ok(
            l,
            r,
            now,
            Command::Claim {
                task: Some(tid(task)),
                agent: aid(agent),
                worker: None,
                ttl: 100,
            },
        );
        match &evs.last().unwrap_or_else(|| panic!("no event")).event {
            Event::Claimed { token, .. } => *token,
            e => panic!("unexpected {e:?}"),
        }
    }

    fn state(l: &Ledger, id: &str) -> State {
        l.task(&tid(id))
            .unwrap_or_else(|| panic!("{id}"))
            .life
            .state()
    }

    #[test]
    fn add_assigns_sequence_numbers_and_keeps_order() {
        let r = reg();
        let mut l = Ledger::new();
        let e1 = ok(&mut l, &r, 10, Command::Add(spec("b", "build", &[])));
        let e2 = ok(&mut l, &r, 11, Command::Add(spec("a", "build", &[])));
        assert_eq!(e1[0].seq, 1);
        assert_eq!(e2[0].seq, 2);
        assert_eq!(e1[0].actor, COORDINATOR);
        assert_eq!(l.next_seq(), 3);
        let order: Vec<&str> = l.tasks().map(|t| t.spec.id.as_str()).collect();
        assert_eq!(order, ["b", "a"]);
        assert_eq!(l.len(), 2);
        assert!(!l.is_empty());
    }

    #[test]
    fn add_rejects_duplicates_unknown_deps_and_unserved_skills() {
        let r = reg();
        let mut l = Ledger::new();
        ok(&mut l, &r, 1, Command::Add(spec("a", "build", &[])));
        assert!(matches!(
            run(&mut l, &r, 2, Command::Add(spec("a", "build", &[]))),
            Err(MeshError::DuplicateTask(_))
        ));
        assert!(matches!(
            run(&mut l, &r, 2, Command::Add(spec("b", "build", &["x"]))),
            Err(MeshError::UnknownDependency { .. })
        ));
        assert!(matches!(
            run(&mut l, &r, 2, Command::Add(spec("c", "dance", &[]))),
            Err(MeshError::UnservedSkill { .. })
        ));
        assert_eq!(l.len(), 1);
    }

    #[test]
    fn plan_is_atomic() {
        let r = reg();
        let mut l = Ledger::new();
        let bad = vec![spec("a", "build", &[]), spec("b", "dance", &["a"])];
        assert!(run(&mut l, &r, 1, Command::Plan(bad)).is_err());
        assert!(l.is_empty());
        let good = vec![spec("b", "review", &["a"]), spec("a", "build", &[])];
        let evs = ok(&mut l, &r, 1, Command::Plan(good));
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0].event.task().as_str(), "a");
    }

    #[test]
    fn readiness_is_derived_from_dependencies() {
        let r = reg();
        let mut l = Ledger::new();
        ok(
            &mut l,
            &r,
            1,
            Command::Plan(vec![
                spec("a", "build", &[]),
                spec("b", "review", &["a"]),
                spec("c", "review", &["a"]),
            ]),
        );
        assert_eq!(l.readiness(&tid("a")), Readiness::Ready);
        assert_eq!(l.readiness(&tid("b")), Readiness::Waiting(vec![tid("a")]));
        let t = claim(&mut l, &r, 2, "a", "builder");
        assert_eq!(l.readiness(&tid("a")), Readiness::NotSubmitted);
        ok(
            &mut l,
            &r,
            3,
            Command::Complete {
                task: tid("a"),
                token: t,
                evidence: "tests pass".into(),
            },
        );
        assert_eq!(l.readiness(&tid("b")), Readiness::Ready);
        let ready: Vec<&str> = l
            .ready_for(&r, Some(&aid("reviewer")))
            .iter()
            .map(|t| t.spec.id.as_str())
            .collect();
        assert_eq!(ready, ["b", "c"]);
        assert!(l.ready_for(&r, Some(&aid("builder"))).is_empty());
        assert_eq!(l.ready_for(&r, None).len(), 2);
    }

    #[test]
    fn blocked_when_a_dependency_failed_or_was_canceled() {
        let r = reg();
        let mut l = Ledger::new();
        ok(
            &mut l,
            &r,
            1,
            Command::Plan(vec![spec("a", "build", &[]), spec("b", "review", &["a"])]),
        );
        ok(
            &mut l,
            &r,
            2,
            Command::Cancel {
                task: tid("a"),
                reason: "obsolete".into(),
            },
        );
        assert_eq!(l.readiness(&tid("b")), Readiness::Blocked(vec![tid("a")]));
        assert!(matches!(
            run(
                &mut l,
                &r,
                3,
                Command::Claim {
                    task: Some(tid("b")),
                    agent: aid("reviewer"),
                    worker: None,
                    ttl: 10
                }
            ),
            Err(MeshError::NotReady { .. })
        ));
    }

    #[test]
    fn claim_checks_agent_skill_wip_and_ttl() {
        let r = reg();
        let mut l = Ledger::new();
        ok(
            &mut l,
            &r,
            1,
            Command::Plan(vec![spec("a", "spec", &[]), spec("b", "spec", &[])]),
        );
        let c = |task: &str, agent: &str, ttl: u64| Command::Claim {
            task: Some(tid(task)),
            agent: aid(agent),
            worker: None,
            ttl,
        };
        assert!(matches!(
            run(&mut l, &r, 2, c("a", "ghost", 10)),
            Err(MeshError::UnknownAgent(_))
        ));
        assert!(matches!(
            run(&mut l, &r, 2, c("a", "builder", 10)),
            Err(MeshError::SkillMismatch { .. })
        ));
        assert!(matches!(
            run(&mut l, &r, 2, c("a", "architect", 0)),
            Err(MeshError::InvalidTtl { .. })
        ));
        assert!(matches!(
            run(&mut l, &r, 2, c("a", "architect", MAX_TTL + 1)),
            Err(MeshError::InvalidTtl { .. })
        ));
        assert!(matches!(
            run(&mut l, &r, 2, c("zz", "architect", 10)),
            Err(MeshError::UnknownTask(_))
        ));
        claim(&mut l, &r, 2, "a", "architect");
        assert!(matches!(
            run(&mut l, &r, 2, c("a", "architect", 10)),
            Err(MeshError::InvalidTransition { .. })
        ));
        assert!(matches!(
            run(&mut l, &r, 2, c("b", "architect", 10)),
            Err(MeshError::WipLimit { limit: 1, .. })
        ));
        assert_eq!(l.active_count(&aid("architect")), 1);
    }

    #[test]
    fn claim_next_takes_the_first_ready_task_for_the_role() {
        let r = reg();
        let mut l = Ledger::new();
        ok(
            &mut l,
            &r,
            1,
            Command::Plan(vec![
                spec("s", "spec", &[]),
                spec("b1", "build", &["s"]),
                spec("b2", "build", &[]),
            ]),
        );
        let evs = ok(
            &mut l,
            &r,
            2,
            Command::Claim {
                task: None,
                agent: aid("builder"),
                worker: Some("w-1".into()),
                ttl: 10,
            },
        );
        match &evs[0].event {
            Event::Claimed { task, worker, .. } => {
                assert_eq!(task.as_str(), "b2");
                assert_eq!(worker, "w-1");
            }
            e => panic!("unexpected {e:?}"),
        }
        assert_eq!(evs[0].actor, "builder");
        assert!(matches!(
            run(
                &mut l,
                &r,
                2,
                Command::Claim {
                    task: None,
                    agent: aid("builder"),
                    worker: None,
                    ttl: 10
                }
            ),
            Err(MeshError::NothingReady(_))
        ));
    }

    #[test]
    fn tokens_increase_across_tasks() {
        let r = reg();
        let mut l = Ledger::new();
        ok(
            &mut l,
            &r,
            1,
            Command::Plan(vec![spec("a", "build", &[]), spec("b", "build", &[])]),
        );
        let t1 = claim(&mut l, &r, 2, "a", "builder");
        let t2 = claim(&mut l, &r, 2, "b", "builder");
        assert!(t2 > t1);
        assert_eq!(l.next_token(), Token(t2.0 + 1));
    }

    #[test]
    fn worker_commands_need_the_current_token() {
        let r = reg();
        let mut l = Ledger::new();
        ok(&mut l, &r, 1, Command::Add(spec("a", "build", &[])));
        let t = claim(&mut l, &r, 2, "a", "builder");
        let stale = Token(t.0 + 5);
        assert!(matches!(
            run(
                &mut l,
                &r,
                3,
                Command::Complete {
                    task: tid("a"),
                    token: stale,
                    evidence: "x".into()
                }
            ),
            Err(MeshError::StaleToken { .. })
        ));
        assert!(matches!(
            run(
                &mut l,
                &r,
                3,
                Command::Heartbeat {
                    task: tid("a"),
                    token: stale,
                    ttl: 10
                }
            ),
            Err(MeshError::StaleToken { .. })
        ));
        assert!(matches!(
            run(
                &mut l,
                &r,
                3,
                Command::Complete {
                    task: tid("a"),
                    token: t,
                    evidence: " ".into()
                }
            ),
            Err(MeshError::EmptyText(_))
        ));
        ok(
            &mut l,
            &r,
            3,
            Command::Heartbeat {
                task: tid("a"),
                token: t,
                ttl: 500,
            },
        );
        assert_eq!(
            l.task(&tid("a"))
                .and_then(|x| x.life.lease())
                .map(|x| x.expires_at),
            Some(503)
        );
        let evs = ok(
            &mut l,
            &r,
            4,
            Command::Complete {
                task: tid("a"),
                token: t,
                evidence: "cargo test ok".into(),
            },
        );
        assert_eq!(evs[0].actor, "builder");
        assert_eq!(state(&l, "a"), State::Completed);
        assert_eq!(
            l.task(&tid("a"))
                .and_then(|x| x.last_note(NoteKind::Evidence)),
            Some("cargo test ok")
        );
    }

    #[test]
    fn expired_lease_rejects_worker_writes_until_reaped() {
        let r = reg();
        let mut l = Ledger::new();
        ok(&mut l, &r, 1, Command::Add(spec("a", "build", &[])));
        let t = claim(&mut l, &r, 10, "a", "builder");
        assert!(matches!(
            run(
                &mut l,
                &r,
                110,
                Command::Complete {
                    task: tid("a"),
                    token: t,
                    evidence: "x".into()
                }
            ),
            Err(MeshError::LeaseExpired(_))
        ));
        assert_eq!(l.overdue(110).len(), 1);
        assert!(l.overdue(109).is_empty());
        let evs = ok(&mut l, &r, 110, Command::Reap);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].actor, SYSTEM);
        assert_eq!(state(&l, "a"), State::Submitted);
        assert_eq!(l.task(&tid("a")).map(|x| x.life.failures()), Some(1));
        assert!(ok(&mut l, &r, 111, Command::Reap).is_empty());
    }

    #[test]
    fn fail_retry_and_budget() {
        let r = reg();
        let mut l = Ledger::new();
        let mut s = spec("a", "build", &[]);
        s.max_failures = 1;
        ok(&mut l, &r, 1, Command::Add(s));
        let t = claim(&mut l, &r, 2, "a", "builder");
        assert!(matches!(
            run(
                &mut l,
                &r,
                3,
                Command::Fail {
                    task: tid("a"),
                    token: t,
                    reason: String::new()
                }
            ),
            Err(MeshError::EmptyText(_))
        ));
        ok(
            &mut l,
            &r,
            3,
            Command::Fail {
                task: tid("a"),
                token: t,
                reason: "tests fail".into(),
            },
        );
        assert_eq!(state(&l, "a"), State::Failed);
        assert!(matches!(
            run(&mut l, &r, 4, Command::Retry { task: tid("a") }),
            Err(MeshError::BudgetExhausted { max: 1, .. })
        ));
        ok(
            &mut l,
            &r,
            5,
            Command::Cancel {
                task: tid("a"),
                reason: "budget used".into(),
            },
        );
        assert!(matches!(
            run(
                &mut l,
                &r,
                6,
                Command::Cancel {
                    task: tid("a"),
                    reason: "again".into()
                }
            ),
            Err(MeshError::InvalidTransition { .. })
        ));
    }

    #[test]
    fn ask_and_answer() {
        let r = reg();
        let mut l = Ledger::new();
        ok(&mut l, &r, 1, Command::Add(spec("a", "build", &[])));
        let t = claim(&mut l, &r, 2, "a", "builder");
        ok(
            &mut l,
            &r,
            3,
            Command::Ask {
                task: tid("a"),
                token: t,
                question: "Which file?".into(),
            },
        );
        assert_eq!(state(&l, "a"), State::InputRequired);
        assert_eq!(l.active_count(&aid("builder")), 0);
        assert!(matches!(
            run(
                &mut l,
                &r,
                4,
                Command::Answer {
                    task: tid("a"),
                    text: String::new()
                }
            ),
            Err(MeshError::EmptyText(_))
        ));
        ok(
            &mut l,
            &r,
            4,
            Command::Answer {
                task: tid("a"),
                text: "src/lib.rs".into(),
            },
        );
        assert_eq!(state(&l, "a"), State::Submitted);
        let task = l.task(&tid("a")).unwrap_or_else(|| panic!("a"));
        assert_eq!(task.last_note(NoteKind::Question), Some("Which file?"));
        assert_eq!(task.last_note(NoteKind::Answer), Some("src/lib.rs"));
        assert!(matches!(
            run(
                &mut l,
                &r,
                5,
                Command::Answer {
                    task: tid("a"),
                    text: "x".into()
                }
            ),
            Err(MeshError::InvalidTransition { .. })
        ));
    }

    #[test]
    fn replaying_the_events_gives_the_same_ledger() {
        let r = reg();
        let mut l = Ledger::new();
        let mut all = ok(
            &mut l,
            &r,
            1,
            Command::Plan(vec![spec("a", "build", &[]), spec("b", "review", &["a"])]),
        );
        let t = claim(&mut l, &r, 2, "a", "builder");
        all.push(Envelope {
            seq: 3,
            ts: 2,
            actor: "builder".into(),
            event: Event::Claimed {
                task: tid("a"),
                agent: aid("builder"),
                worker: "builder".into(),
                token: t,
                expires_at: 102,
            },
        });
        all.extend(ok(
            &mut l,
            &r,
            3,
            Command::Complete {
                task: tid("a"),
                token: t,
                evidence: "ok".into(),
            },
        ));
        let mut copy = Ledger::new();
        for env in &all {
            copy.apply(env).unwrap_or_else(|e| panic!("{e}"));
        }
        assert_eq!(copy, l);
    }

    #[test]
    fn apply_rejects_reused_tokens_and_observe_keeps_numbers_fresh() {
        let r = reg();
        let mut l = Ledger::new();
        ok(
            &mut l,
            &r,
            1,
            Command::Plan(vec![spec("a", "build", &[]), spec("b", "build", &[])]),
        );
        claim(&mut l, &r, 2, "a", "builder");
        let reused = Envelope {
            seq: 9,
            ts: 3,
            actor: "builder".into(),
            event: Event::Claimed {
                task: tid("b"),
                agent: aid("builder"),
                worker: "w".into(),
                token: Token(1),
                expires_at: 50,
            },
        };
        let before = l.clone();
        assert!(matches!(
            l.apply(&reused),
            Err(MeshError::TokenReused { .. })
        ));
        assert_eq!(l, before);
        let ghost = Envelope {
            seq: 40,
            ts: 3,
            actor: "builder".into(),
            event: Event::Claimed {
                task: tid("zz"),
                agent: aid("builder"),
                worker: "w".into(),
                token: Token(77),
                expires_at: 50,
            },
        };
        l.observe(&ghost);
        assert!(l.apply(&ghost).is_err());
        assert_eq!(l.next_seq(), 41);
        assert_eq!(l.next_token(), Token(78));
    }
}

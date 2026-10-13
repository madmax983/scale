//! Agent mesh for the AI agents that build SCALE.
//!
//! The coordinator (the main chat session) plans tasks. Workers (subagents) do them.
//! The mesh records every task, claim, and result in an append-only ledger.
//!
//! - [`registry`]: agent cards (roles, skills, write scopes).
//! - [`lifecycle`]: task states, leases, and fencing tokens.
//! - [`ledger`]: state rebuilt from events, and the commands.
//! - [`store`]: the `.mesh/` folder, file lock, and replay.
//! - [`scope`] and [`hook`]: write-scope checks.
//! - [`board`] and [`doctor`]: generated views and health checks.

pub mod board;
pub mod doctor;
pub mod error;
pub mod event;
pub mod hook;
pub mod ids;
pub mod ledger;
pub mod lifecycle;
pub mod plan;
pub mod registry;
pub mod scope;
pub mod store;
pub mod time;

pub use error::{MeshError, Result};
pub use event::{Envelope, Event, TaskSpec};
pub use ids::{AgentId, Skill, TaskId, Token};
pub use ledger::{Command, Ctx, Ledger, Task};
pub use lifecycle::{Lease, Lifecycle, State};
pub use registry::{AgentCard, Registry};
pub use store::Store;

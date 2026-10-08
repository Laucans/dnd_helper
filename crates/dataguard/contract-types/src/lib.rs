//! Serde types for the DataGuard contracts G (queue entry), H (command
//! result), J (impact plan) and L (message).
//!
//! Pure data: no I/O, no clock, no database. Each type enforces on
//! deserialization the constraints of its schema in `contracts/`, and in a few
//! documented places is stricter than the schema.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod command_result;
mod error;
mod impact_plan;
mod message;
mod queue_entry;
mod text;

pub use command_result::{CommandResult, CommandStatus, Outcome};
pub use error::ContractError;
pub use impact_plan::{
    CascadeItem, CascadePolicy, Classification, Decision, Dependencies, Hold, ImpactPlan,
    ImpactPlanFields, InvariantsAfter,
};
pub use message::{CommandApplied, CommandQueued, CommandRejected, InvariantAtRisk, Message};
pub use queue_entry::{
    BasedOn, Confirmation, OnExpire, Parked, PendingAhead, Projection, QueueEntry, QueueState,
};
pub use text::{
    Actor, CommandId, DataCapabilityRef, Partition, ReaderRef, Recipients, Ttl, Violation,
    Violations,
};

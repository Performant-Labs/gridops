//! `gridops-host-agent`: the portable agent that joins a machine to a `GridOps`
//! fleet.
//!
//! The agent connects outbound to the API, keeps a session with handshakes,
//! inventory and heartbeats ([`supervisor`]), journals every command before
//! acting on it ([`journal`]), and passes typed, ownership-checked operations
//! to a privileged runtime helper ([`dispatch`], [`helper`]). It never accepts
//! inbound connections or arbitrary commands.

mod activity;
pub mod client;
pub mod dispatch;
pub mod docker;
pub mod helper;
pub mod journal;
pub mod logs;
pub mod native;
mod output;
pub mod probe;
pub mod runtime;
pub mod supervisor;

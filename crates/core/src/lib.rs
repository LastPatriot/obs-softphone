// SPDX-License-Identifier: GPL-2.0-or-later
//! Call-control logic for the OBS softphone (DESIGN.md §3.5).
//!
//! Everything here is plain Rust with no PJSIP, OBS or Qt code. The outside
//! world is reached only through the three ports in [`ports`], so the whole
//! state machine is unit-tested with fakes.

pub mod backoff;
pub mod caller_id;
pub mod line;
pub mod ports;
pub mod runtime;

pub use line::{Line, LineConfig};
pub use ports::{Binding, CallId, Clock, Event, LineView, SipControl, Status, TimerId, Ui};

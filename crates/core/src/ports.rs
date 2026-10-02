// SPDX-License-Identifier: GPL-2.0-or-later
//! The only interfaces between the core and the outside world.

use std::time::Duration;

/// PJSIP's call id (`pjsua_call_id`).
pub type CallId = i32;

/// Commands the core sends to the SIP stack. Results come back as [`Event`]s.
pub trait SipControl {
    /// Start (or restart) registration of the account.
    fn register(&mut self);
    /// Stop registering and stop all refreshes. Harmless if our binding is
    /// already gone (it only removes our own contact).
    fn unregister(&mut self);
    fn answer(&mut self, call: CallId);
    fn hangup(&mut self, call: CallId);
    /// Reject with `486 Busy Here`.
    fn reject_busy(&mut self, call: CallId);
    /// REGISTER without a Contact header: ask the server who holds the account,
    /// without changing anything (RFC 3261 §10.2.4). Answered by
    /// [`Event::Bindings`].
    fn query_bindings(&mut self);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TimerId {
    AutoAnswer,
    Reconnect,
    OptionsWatchdog,
}

/// Monotonic time and one-shot timers. Starting a timer that is already
/// running restarts it. Expiry arrives as [`Event::Timer`].
pub trait Clock {
    fn now(&self) -> Duration;
    fn start_timer(&mut self, id: TimerId, after: Duration);
    fn cancel_timer(&mut self, id: TimerId);
}

/// Receives the line's state for display (dock, logs, console).
pub trait Ui {
    fn show(&mut self, view: &LineView);
}

/// What a bindings query found for the account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Binding {
    /// Our contact is still the registered one.
    Ours,
    /// Another device holds the account.
    Other,
    /// Nobody is registered.
    None,
    /// The query failed (timeout, transport error).
    Unknown,
}

/// Everything that can happen to the line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    // From settings / host.
    Enable,
    Disable,
    SetAutoAnswer(bool),
    // From the dock.
    Answer,
    EndCall,
    Retry,
    TakeBack,
    // From SIP.
    Registered,
    /// Unregistration completed (after [`SipControl::unregister`]).
    Unregistered,
    RegistrationFailed { code: u16, reason: String },
    TransportDown,
    IncomingCall { call: CallId, remote: String },
    CallConfirmed { call: CallId },
    CallEnded { call: CallId },
    OptionsReceived,
    Bindings(Binding),
    // From the clock.
    Timer(TimerId),
}

/// The states shown to the host (DESIGN.md §4.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Disabled,
    Connecting,
    Ready,
    Ringing,
    OnAir,
    Error,
    Replaced,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineView {
    pub status: Status,
    pub caller: Option<String>,
    /// Clock time the call went on air.
    pub on_air_since: Option<Duration>,
    /// Plain-language detail, e.g. the reason for an error.
    pub message: Option<String>,
    pub auto_answer: bool,
}

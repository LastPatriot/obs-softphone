// SPDX-License-Identifier: GPL-2.0-or-later
//! The studio line: registration, calls and Replaced detection (DESIGN.md §4).
//!
//! Registration and the call are tracked separately, so a caller is never
//! refused just because a registration refresh is in flight. The displayed
//! [`Status`] is derived from both.

use std::time::Duration;

use crate::backoff::Backoff;
use crate::caller_id;
use crate::ports::{Binding, CallId, Clock, Event, LineView, SipControl, Status, TimerId, Ui};

#[derive(Clone, Debug)]
pub struct LineConfig {
    pub auto_answer: bool,
    pub auto_answer_delay: Duration,
    /// Watch for missing OPTIONS and confirm with a bindings query (§4.4).
    pub replaced_detection: bool,
    pub options_timeout: Duration,
    /// Minimum gap between two "Take the line back" presses.
    pub take_back_cooldown: Duration,
    pub backoff: Backoff,
}

impl Default for LineConfig {
    fn default() -> Self {
        Self {
            auto_answer: true,
            auto_answer_delay: Duration::from_millis(300),
            replaced_detection: true,
            options_timeout: Duration::from_secs(75),
            take_back_cooldown: Duration::from_secs(10),
            backoff: Backoff::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Reg {
    Off,
    Connecting,
    /// `verifying`: a bindings query is outstanding.
    Registered { verifying: bool },
    Failed { reason: String, retrying: bool },
    Replaced,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Phase {
    Ringing,
    OnAir { since: Duration },
}

#[derive(Clone, Debug)]
struct Call {
    id: CallId,
    caller: String,
    phase: Phase,
}

pub struct Line<S, C, U> {
    cfg: LineConfig,
    sip: S,
    clock: C,
    ui: U,
    reg: Reg,
    call: Option<Call>,
    last_take_back: Option<Duration>,
    shown: Option<LineView>,
}

impl<S: SipControl, C: Clock, U: Ui> Line<S, C, U> {
    pub fn new(cfg: LineConfig, sip: S, clock: C, ui: U) -> Self {
        let mut line = Self {
            cfg,
            sip,
            clock,
            ui,
            reg: Reg::Off,
            call: None,
            last_take_back: None,
            shown: None,
        };
        line.publish();
        line
    }

    pub fn view(&self) -> LineView {
        let (status, message) = match (&self.reg, &self.call) {
            (Reg::Off, _) => (Status::Disabled, None),
            (_, Some(Call { phase: Phase::Ringing, .. })) => (Status::Ringing, None),
            (_, Some(Call { phase: Phase::OnAir { .. }, .. })) => (Status::OnAir, None),
            (Reg::Connecting, None) => (Status::Connecting, None),
            (Reg::Registered { .. }, None) => (Status::Ready, None),
            (Reg::Failed { reason, retrying }, None) => {
                let msg = if *retrying { format!("{reason} (retrying)") } else { reason.clone() };
                (Status::Error, Some(msg))
            }
            (Reg::Replaced, None) => (
                Status::Replaced,
                Some("Another device has signed in with this account".to_string()),
            ),
        };
        LineView {
            status,
            caller: self.call.as_ref().map(|c| c.caller.clone()),
            on_air_since: match self.call {
                Some(Call { phase: Phase::OnAir { since }, .. }) => Some(since),
                _ => None,
            },
            message,
            auto_answer: self.cfg.auto_answer,
        }
    }

    pub fn handle(&mut self, event: Event) {
        match event {
            Event::Enable => self.enable(),
            Event::Disable => self.disable(),
            Event::SetAutoAnswer(on) => self.set_auto_answer(on),
            Event::Answer => self.answer_ringing(),
            Event::EndCall => {
                if let Some(call) = &self.call {
                    self.sip.hangup(call.id);
                }
            }
            Event::Retry => {
                if matches!(self.reg, Reg::Failed { .. } | Reg::Connecting) {
                    self.connect();
                }
            }
            Event::TakeBack => self.take_back(),
            Event::Registered => self.registered(),
            Event::Unregistered => {}
            Event::RegistrationFailed { code, reason } => self.registration_failed(code, reason),
            Event::TransportDown => self.transport_down(),
            Event::IncomingCall { call, remote } => self.incoming(call, &remote),
            Event::CallConfirmed { call } => {
                let now = self.clock.now();
                if let Some(c) = self.call.as_mut().filter(|c| c.id == call) {
                    c.phase = Phase::OnAir { since: now };
                }
            }
            Event::CallEnded { call } => {
                if self.call.as_ref().is_some_and(|c| c.id == call) {
                    self.call = None;
                    self.clock.cancel_timer(TimerId::AutoAnswer);
                }
            }
            Event::OptionsReceived => {
                if matches!(self.reg, Reg::Registered { verifying: false }) {
                    self.arm_watchdog();
                }
            }
            Event::Bindings(binding) => self.bindings(binding),
            Event::Timer(id) => self.timer(id),
        }
        self.publish();
    }

    fn enable(&mut self) {
        if self.reg == Reg::Off {
            self.cfg.backoff.reset();
            self.connect();
        }
    }

    fn disable(&mut self) {
        if self.reg == Reg::Off {
            return;
        }
        // BYE first, then unregister (DESIGN.md §4.2).
        if let Some(call) = self.call.take() {
            self.sip.hangup(call.id);
        }
        for id in [TimerId::AutoAnswer, TimerId::Reconnect, TimerId::OptionsWatchdog] {
            self.clock.cancel_timer(id);
        }
        self.sip.unregister();
        self.reg = Reg::Off;
    }

    fn set_auto_answer(&mut self, on: bool) {
        self.cfg.auto_answer = on;
        let ringing = matches!(self.call, Some(Call { phase: Phase::Ringing, .. }));
        match (on, ringing) {
            (true, true) => self.clock.start_timer(TimerId::AutoAnswer, self.cfg.auto_answer_delay),
            (false, _) => self.clock.cancel_timer(TimerId::AutoAnswer),
            _ => {}
        }
    }

    fn connect(&mut self) {
        self.clock.cancel_timer(TimerId::Reconnect);
        self.clock.cancel_timer(TimerId::OptionsWatchdog);
        self.reg = Reg::Connecting;
        self.sip.register();
    }

    fn registered(&mut self) {
        if matches!(self.reg, Reg::Off | Reg::Replaced) {
            return;
        }
        self.cfg.backoff.reset();
        self.clock.cancel_timer(TimerId::Reconnect);
        if !matches!(self.reg, Reg::Registered { .. }) {
            self.reg = Reg::Registered { verifying: false };
            self.arm_watchdog();
        }
    }

    fn registration_failed(&mut self, code: u16, reason: String) {
        if matches!(self.reg, Reg::Off | Reg::Replaced) {
            return;
        }
        self.clock.cancel_timer(TimerId::OptionsWatchdog);
        // Wrong password or IP not allowed: retrying won't help; wait for "Retry".
        let retrying = !matches!(code, 401 | 403 | 407);
        if retrying {
            let delay = self.cfg.backoff.next_delay();
            self.clock.start_timer(TimerId::Reconnect, delay);
        }
        self.reg = Reg::Failed { reason: plain_reason(code, &reason), retrying };
    }

    fn transport_down(&mut self) {
        // In-dialog requests can't reach us on a new connection (§4.3).
        if let Some(call) = self.call.take() {
            self.sip.hangup(call.id);
            self.clock.cancel_timer(TimerId::AutoAnswer);
        }
        // While connecting, the failing REGISTER reports itself and the
        // backoff applies; reconnecting here too would loop on a dead network.
        if matches!(self.reg, Reg::Registered { .. }) {
            self.connect();
        }
    }

    fn incoming(&mut self, id: CallId, remote: &str) {
        if self.reg == Reg::Off || self.call.is_some() {
            self.sip.reject_busy(id);
            return;
        }
        self.call = Some(Call { id, caller: caller_id::display_name(remote), phase: Phase::Ringing });
        if self.cfg.auto_answer {
            self.clock.start_timer(TimerId::AutoAnswer, self.cfg.auto_answer_delay);
        }
    }

    fn answer_ringing(&mut self) {
        if let Some(Call { id, phase: Phase::Ringing, .. }) = self.call {
            self.clock.cancel_timer(TimerId::AutoAnswer);
            self.sip.answer(id);
        }
    }

    fn take_back(&mut self) {
        if self.reg != Reg::Replaced {
            return;
        }
        let now = self.clock.now();
        if self.last_take_back.is_some_and(|t| now < t + self.cfg.take_back_cooldown) {
            return;
        }
        self.last_take_back = Some(now);
        self.connect();
    }

    fn arm_watchdog(&mut self) {
        if self.cfg.replaced_detection {
            self.clock.start_timer(TimerId::OptionsWatchdog, self.cfg.options_timeout);
        }
    }

    fn bindings(&mut self, binding: Binding) {
        if self.reg != (Reg::Registered { verifying: true }) {
            return;
        }
        match binding {
            Binding::Ours | Binding::Unknown => {
                self.reg = Reg::Registered { verifying: false };
                self.arm_watchdog();
            }
            Binding::Other => {
                // Stop refreshing so we don't knock the other device off (§4.4).
                self.sip.unregister();
                self.reg = Reg::Replaced;
            }
            Binding::None => self.connect(),
        }
    }

    fn timer(&mut self, id: TimerId) {
        match id {
            TimerId::AutoAnswer => {
                if self.cfg.auto_answer {
                    self.answer_ringing();
                }
            }
            TimerId::Reconnect => {
                if matches!(self.reg, Reg::Failed { retrying: true, .. } | Reg::Connecting) {
                    self.connect();
                }
            }
            TimerId::OptionsWatchdog => {
                if self.reg == (Reg::Registered { verifying: false }) {
                    self.reg = Reg::Registered { verifying: true };
                    self.sip.query_bindings();
                }
            }
        }
    }

    fn publish(&mut self) {
        let view = self.view();
        if self.shown.as_ref() != Some(&view) {
            self.ui.show(&view);
            self.shown = Some(view);
        }
    }
}

fn plain_reason(code: u16, reason: &str) -> String {
    let lower = reason.to_ascii_lowercase();
    let plain = match code {
        401 | 407 => "Wrong username or password",
        403 => "Server refused the login (password or allowed IP)",
        404 => "Extension not found on the server",
        408 => "Server did not answer",
        502 | 503 if lower.contains("certificate") => "Server's TLS certificate was not accepted",
        502 | 503 if lower.contains("refused") => "Server refused the connection (port closed or IP not allowed)",
        502 | 503 => "Can't reach the server",
        _ => "",
    };
    match (plain.is_empty(), reason.is_empty()) {
        (true, true) => format!("Registration failed ({code})"),
        (true, false) => format!("Registration failed: {reason} ({code})"),
        // Transport errors: keep pjlib's detail, it's the only clue.
        (false, false) if matches!(code, 502 | 503) => format!("{plain}: {reason}"),
        (false, _) => format!("{plain} ({code})"),
    }
}

#[cfg(test)]
mod tests;

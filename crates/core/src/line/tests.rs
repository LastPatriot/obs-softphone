// SPDX-License-Identifier: GPL-2.0-or-later
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Cmd {
    Register,
    Unregister,
    Answer(CallId),
    Hangup(CallId),
    Busy(CallId),
    QueryBindings,
}

#[derive(Default)]
struct World {
    cmds: Vec<Cmd>,
    now: Duration,
    timers: BTreeMap<TimerId, Duration>,
    views: Vec<LineView>,
}

type Shared = Rc<RefCell<World>>;

struct FakeSip(Shared);
impl SipControl for FakeSip {
    fn register(&mut self) {
        self.0.borrow_mut().cmds.push(Cmd::Register);
    }
    fn unregister(&mut self) {
        self.0.borrow_mut().cmds.push(Cmd::Unregister);
    }
    fn answer(&mut self, call: CallId) {
        self.0.borrow_mut().cmds.push(Cmd::Answer(call));
    }
    fn hangup(&mut self, call: CallId) {
        self.0.borrow_mut().cmds.push(Cmd::Hangup(call));
    }
    fn reject_busy(&mut self, call: CallId) {
        self.0.borrow_mut().cmds.push(Cmd::Busy(call));
    }
    fn query_bindings(&mut self) {
        self.0.borrow_mut().cmds.push(Cmd::QueryBindings);
    }
}

struct FakeClock(Shared);
impl Clock for FakeClock {
    fn now(&self) -> Duration {
        self.0.borrow().now
    }
    fn start_timer(&mut self, id: TimerId, after: Duration) {
        let mut w = self.0.borrow_mut();
        let at = w.now + after;
        w.timers.insert(id, at);
    }
    fn cancel_timer(&mut self, id: TimerId) {
        self.0.borrow_mut().timers.remove(&id);
    }
}

struct FakeUi(Shared);
impl Ui for FakeUi {
    fn show(&mut self, view: &LineView) {
        self.0.borrow_mut().views.push(view.clone());
    }
}

struct Harness {
    w: Shared,
    line: Line<FakeSip, FakeClock, FakeUi>,
}

impl Harness {
    fn new() -> Self {
        Self::with(LineConfig::default())
    }

    fn with(cfg: LineConfig) -> Self {
        let w: Shared = Rc::default();
        let line = Line::new(cfg, FakeSip(w.clone()), FakeClock(w.clone()), FakeUi(w.clone()));
        Self { w, line }
    }

    /// Enabled and registered, commands cleared.
    fn ready() -> Self {
        let mut h = Self::new();
        h.send(Event::Enable);
        h.send(Event::Registered);
        h.take_cmds();
        h
    }

    fn send(&mut self, e: Event) {
        self.line.handle(e);
    }

    /// Moves the clock forward, firing due timers in order.
    fn advance(&mut self, by: Duration) {
        let end = self.w.borrow().now + by;
        loop {
            let due = {
                let w = self.w.borrow();
                w.timers.iter().map(|(id, at)| (*at, *id)).filter(|(at, _)| *at <= end).min()
            };
            let Some((at, id)) = due else { break };
            {
                let mut w = self.w.borrow_mut();
                w.now = at;
                w.timers.remove(&id);
            }
            self.send(Event::Timer(id));
        }
        self.w.borrow_mut().now = end;
    }

    fn secs(&mut self, s: u64) {
        self.advance(Duration::from_secs(s));
    }

    fn take_cmds(&mut self) -> Vec<Cmd> {
        std::mem::take(&mut self.w.borrow_mut().cmds)
    }

    fn status(&self) -> Status {
        self.line.view().status
    }

    fn timer_running(&self, id: TimerId) -> bool {
        self.w.borrow().timers.contains_key(&id)
    }

    fn incoming(&mut self, id: CallId) {
        self.send(Event::IncomingCall { call: id, remote: r#""Mrs Ade" <sip:callin@x>"#.into() });
    }
}

fn failed(code: u16) -> Event {
    Event::RegistrationFailed { code, reason: String::new() }
}

// --- registration ----------------------------------------------------------

#[test]
fn starts_disabled_and_registers_on_enable() {
    let mut h = Harness::new();
    assert_eq!(h.status(), Status::Disabled);
    h.send(Event::Enable);
    assert_eq!(h.status(), Status::Connecting);
    assert_eq!(h.take_cmds(), [Cmd::Register]);
    h.send(Event::Registered);
    assert_eq!(h.status(), Status::Ready);
}

#[test]
fn refreshes_do_not_change_anything() {
    let mut h = Harness::ready();
    h.send(Event::Registered);
    assert_eq!(h.status(), Status::Ready);
    assert!(h.take_cmds().is_empty());
}

#[test]
fn transient_failure_retries_with_backoff_until_success() {
    let mut h = Harness::new();
    h.send(Event::Enable);
    h.take_cmds();

    let mut gaps = vec![];
    for _ in 0..6 {
        h.send(failed(503));
        assert_eq!(h.status(), Status::Error);
        let mut waited = 0;
        while h.take_cmds().is_empty() {
            h.secs(1);
            waited += 1;
        }
        gaps.push(waited);
    }
    assert_eq!(gaps, [2, 4, 8, 16, 30, 30]);

    h.send(Event::Registered);
    assert_eq!(h.status(), Status::Ready);
    // Backoff resets after success.
    h.send(failed(408));
    h.secs(2);
    assert_eq!(h.take_cmds(), [Cmd::Register]);
}

#[test]
fn auth_failure_waits_for_retry() {
    let mut h = Harness::new();
    h.send(Event::Enable);
    h.take_cmds();
    h.send(failed(401));
    assert_eq!(h.status(), Status::Error);
    assert!(h.line.view().message.unwrap().contains("password"));
    h.secs(600);
    assert!(h.take_cmds().is_empty());

    h.send(Event::Retry);
    assert_eq!(h.status(), Status::Connecting);
    assert_eq!(h.take_cmds(), [Cmd::Register]);
}

#[test]
fn transport_errors_are_explained() {
    let mut h = Harness::new();
    h.send(Event::Enable);
    let msg = |h: &mut Harness, reason: &str| {
        h.send(Event::RegistrationFailed { code: 503, reason: reason.into() });
        h.line.view().message.unwrap()
    };
    assert!(msg(&mut h, "SSL certificate verification error").starts_with("Server's TLS certificate was not accepted"));
    assert!(msg(&mut h, "Connection refused").starts_with("Server refused the connection"));
    assert_eq!(msg(&mut h, "Network is unreachable"), "Can't reach the server: Network is unreachable (retrying)");
}

#[test]
fn retry_is_ignored_when_ready() {
    let mut h = Harness::ready();
    h.send(Event::Retry);
    assert!(h.take_cmds().is_empty());
}

#[test]
fn disable_unregisters_and_stops_retrying() {
    let mut h = Harness::new();
    h.send(Event::Enable);
    h.send(failed(503));
    h.take_cmds();
    h.send(Event::Disable);
    assert_eq!(h.take_cmds(), [Cmd::Unregister]);
    assert_eq!(h.status(), Status::Disabled);
    h.secs(120);
    assert!(h.take_cmds().is_empty());
    // A late result from the stack is ignored.
    h.send(Event::Registered);
    assert_eq!(h.status(), Status::Disabled);
}

// --- calls -----------------------------------------------------------------

#[test]
fn auto_answers_after_delay_then_goes_on_air() {
    let mut h = Harness::ready();
    h.incoming(7);
    let v = h.line.view();
    assert_eq!(v.status, Status::Ringing);
    assert_eq!(v.caller.as_deref(), Some("Mrs Ade"));

    h.advance(Duration::from_millis(299));
    assert!(h.take_cmds().is_empty());
    h.advance(Duration::from_millis(1));
    assert_eq!(h.take_cmds(), [Cmd::Answer(7)]);

    h.secs(1);
    h.send(Event::CallConfirmed { call: 7 });
    let v = h.line.view();
    assert_eq!(v.status, Status::OnAir);
    assert_eq!(v.on_air_since, Some(Duration::from_millis(1300)));

    h.send(Event::CallEnded { call: 7 });
    assert_eq!(h.status(), Status::Ready);
    assert_eq!(h.line.view().caller, None);
}

#[test]
fn manual_answer_when_auto_answer_off() {
    let mut h = Harness::ready();
    h.send(Event::SetAutoAnswer(false));
    h.incoming(1);
    h.secs(10);
    assert!(h.take_cmds().is_empty());
    h.send(Event::Answer);
    assert_eq!(h.take_cmds(), [Cmd::Answer(1)]);
}

#[test]
fn turning_auto_answer_on_while_ringing_answers() {
    let mut h = Harness::ready();
    h.send(Event::SetAutoAnswer(false));
    h.incoming(1);
    h.send(Event::SetAutoAnswer(true));
    h.secs(1);
    assert_eq!(h.take_cmds(), [Cmd::Answer(1)]);
}

#[test]
fn second_call_gets_busy() {
    let mut h = Harness::ready();
    h.incoming(1);
    h.incoming(2);
    assert_eq!(h.take_cmds(), [Cmd::Busy(2)]);
    h.send(Event::CallConfirmed { call: 1 });
    h.incoming(3);
    assert_eq!(h.take_cmds(), [Cmd::Busy(3)]);
    assert_eq!(h.status(), Status::OnAir);
}

#[test]
fn caller_hanging_up_while_ringing_cancels_auto_answer() {
    let mut h = Harness::ready();
    h.incoming(1);
    h.send(Event::CallEnded { call: 1 });
    h.secs(1);
    assert!(h.take_cmds().is_empty());
    assert_eq!(h.status(), Status::Ready);
}

#[test]
fn events_for_other_calls_are_ignored() {
    let mut h = Harness::ready();
    h.incoming(1);
    h.send(Event::CallConfirmed { call: 9 });
    h.send(Event::CallEnded { call: 9 });
    assert_eq!(h.status(), Status::Ringing);
}

#[test]
fn end_call_hangs_up_and_waits_for_the_stack() {
    let mut h = Harness::ready();
    h.incoming(1);
    h.send(Event::CallConfirmed { call: 1 });
    h.take_cmds();
    h.send(Event::EndCall);
    assert_eq!(h.take_cmds(), [Cmd::Hangup(1)]);
    assert_eq!(h.status(), Status::OnAir);
    h.send(Event::CallEnded { call: 1 });
    assert_eq!(h.status(), Status::Ready);
}

#[test]
fn calls_are_accepted_while_a_refresh_is_failing() {
    let mut h = Harness::ready();
    h.send(failed(503));
    h.incoming(4);
    assert_eq!(h.status(), Status::Ringing);
    h.secs(1);
    assert!(h.take_cmds().contains(&Cmd::Answer(4)));
}

#[test]
fn disable_while_on_air_hangs_up_before_unregistering() {
    let mut h = Harness::ready();
    h.incoming(1);
    h.send(Event::CallConfirmed { call: 1 });
    h.take_cmds();
    h.send(Event::Disable);
    assert_eq!(h.take_cmds(), [Cmd::Hangup(1), Cmd::Unregister]);
    assert_eq!(h.status(), Status::Disabled);
}

#[test]
fn incoming_while_disabled_is_rejected() {
    let mut h = Harness::new();
    h.incoming(1);
    assert_eq!(h.take_cmds(), [Cmd::Busy(1)]);
}

#[test]
fn transport_drop_mid_call_hangs_up_and_reconnects() {
    let mut h = Harness::ready();
    h.incoming(1);
    h.send(Event::CallConfirmed { call: 1 });
    h.take_cmds();
    h.send(Event::TransportDown);
    assert_eq!(h.take_cmds(), [Cmd::Hangup(1), Cmd::Register]);
    assert_eq!(h.status(), Status::Connecting);
}

#[test]
fn transport_drop_while_connecting_leaves_retry_to_the_backoff() {
    let mut h = Harness::new();
    h.send(Event::Enable);
    h.take_cmds();
    h.send(Event::TransportDown); // e.g. TLS handshake failed
    assert!(h.take_cmds().is_empty());
    h.send(failed(503));
    h.secs(1);
    assert!(h.take_cmds().is_empty());
    h.secs(1);
    assert_eq!(h.take_cmds(), [Cmd::Register]);
}

// --- Replaced detection (§4.4)---------------------------------------------

#[test]
fn options_keep_the_watchdog_quiet() {
    let mut h = Harness::ready();
    for _ in 0..10 {
        h.secs(30);
        h.send(Event::OptionsReceived);
    }
    assert!(h.take_cmds().is_empty());
    assert_eq!(h.status(), Status::Ready);
}

#[test]
fn missing_options_trigger_a_bindings_query_not_a_verdict() {
    let mut h = Harness::ready();
    h.secs(74);
    assert!(h.take_cmds().is_empty());
    h.secs(1);
    assert_eq!(h.take_cmds(), [Cmd::QueryBindings]);
    assert_eq!(h.status(), Status::Ready);
}

#[test]
fn bindings_ours_means_network_blip() {
    let mut h = Harness::ready();
    h.secs(75);
    h.take_cmds();
    h.send(Event::Bindings(Binding::Ours));
    assert_eq!(h.status(), Status::Ready);
    // Watchdog re-armed.
    h.secs(75);
    assert_eq!(h.take_cmds(), [Cmd::QueryBindings]);
}

#[test]
fn bindings_unknown_keeps_ready_and_rechecks() {
    let mut h = Harness::ready();
    h.secs(75);
    h.take_cmds();
    h.send(Event::Bindings(Binding::Unknown));
    assert_eq!(h.status(), Status::Ready);
    assert!(h.timer_running(TimerId::OptionsWatchdog));
}

#[test]
fn bindings_other_means_replaced_and_registration_stops() {
    let mut h = Harness::ready();
    h.secs(75);
    h.take_cmds();
    h.send(Event::Bindings(Binding::Other));
    assert_eq!(h.status(), Status::Replaced);
    assert_eq!(h.take_cmds(), [Cmd::Unregister]);

    // Nothing more is sent on its own, however long we wait.
    h.secs(3600);
    assert!(h.take_cmds().is_empty());
    // Late stack results don't pull us out of Replaced.
    h.send(Event::Registered);
    h.send(failed(503));
    assert_eq!(h.status(), Status::Replaced);
}

#[test]
fn bindings_none_re_registers() {
    let mut h = Harness::ready();
    h.secs(75);
    h.take_cmds();
    h.send(Event::Bindings(Binding::None));
    assert_eq!(h.take_cmds(), [Cmd::Register]);
    assert_eq!(h.status(), Status::Connecting);
}

#[test]
fn take_back_is_rate_limited() {
    // Short watchdog so the other device can win again within the cooldown.
    let mut h = Harness::with(LineConfig { options_timeout: Duration::from_secs(2), ..LineConfig::default() });
    h.send(Event::Enable);
    h.send(Event::Registered);
    let replace = |h: &mut Harness| {
        h.secs(2);
        h.send(Event::Bindings(Binding::Other));
        assert_eq!(h.status(), Status::Replaced);
        h.take_cmds();
    };

    replace(&mut h);
    h.send(Event::TakeBack);
    assert_eq!(h.take_cmds(), [Cmd::Register]);
    h.send(Event::Registered);

    // Taken again 2 s later: a second take-back inside 10 s is refused.
    replace(&mut h);
    h.send(Event::TakeBack);
    assert!(h.take_cmds().is_empty());
    assert_eq!(h.status(), Status::Replaced);

    // After the cooldown it is allowed again.
    h.secs(8);
    h.send(Event::TakeBack);
    assert_eq!(h.take_cmds(), [Cmd::Register]);
}

#[test]
fn failed_take_back_falls_back_to_normal_retry() {
    let mut h = Harness::ready();
    h.secs(75);
    h.send(Event::Bindings(Binding::Other));
    h.take_cmds();
    h.send(Event::TakeBack);
    h.send(failed(503));
    h.take_cmds();
    assert_eq!(h.status(), Status::Error);
    // From Error, the normal backoff takes over.
    h.send(Event::TakeBack);
    assert!(h.take_cmds().is_empty());
    h.secs(2);
    assert_eq!(h.take_cmds(), [Cmd::Register]);
}

#[test]
fn take_back_only_in_replaced() {
    let mut h = Harness::ready();
    h.send(Event::TakeBack);
    assert!(h.take_cmds().is_empty());
}

#[test]
fn replaced_during_a_call_keeps_the_call() {
    let mut h = Harness::ready();
    h.incoming(1);
    h.send(Event::CallConfirmed { call: 1 });
    h.secs(75);
    h.send(Event::Bindings(Binding::Other));
    assert_eq!(h.status(), Status::OnAir);
    h.send(Event::CallEnded { call: 1 });
    assert_eq!(h.status(), Status::Replaced);
}

#[test]
fn detection_can_be_switched_off() {
    let mut h = Harness::with(LineConfig { replaced_detection: false, ..LineConfig::default() });
    h.send(Event::Enable);
    h.send(Event::Registered);
    h.take_cmds();
    h.secs(3600);
    assert!(h.take_cmds().is_empty());
}

// --- UI --------------------------------------------------------------------

#[test]
fn ui_sees_each_change_once() {
    let mut h = Harness::new();
    h.send(Event::Enable);
    h.send(Event::Registered);
    h.send(Event::Registered);
    h.send(Event::OptionsReceived);
    let statuses: Vec<Status> = h.w.borrow().views.iter().map(|v| v.status).collect();
    assert_eq!(statuses, [Status::Disabled, Status::Connecting, Status::Ready]);
}

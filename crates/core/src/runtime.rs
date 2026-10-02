// SPDX-License-Identifier: GPL-2.0-or-later
//! Runs a [`Line`] on its own thread, fed by one event queue (DESIGN.md §3.5).
//!
//! PJSIP callbacks, OBS signals and dock clicks never call into the core;
//! they post events through an [`EventSink`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::line::{Line, LineConfig};
use crate::ports::{Clock, Event, SipControl, TimerId, Ui};

enum Msg {
    Event(Event),
    Shutdown,
}

/// Cloneable, thread-safe handle for posting events to the line.
#[derive(Clone)]
pub struct EventSink(Sender<Msg>);

impl EventSink {
    /// Returns false once the runtime has stopped.
    pub fn send(&self, event: Event) -> bool {
        self.0.send(Msg::Event(event)).is_ok()
    }
}

pub struct Runtime {
    sink: EventSink,
    thread: Option<JoinHandle<()>>,
}

impl Runtime {
    /// Creates the queue, builds the SIP adapter and UI with a sink so they
    /// can post events, and starts the line thread.
    ///
    /// `make_sip` runs on the line thread, so the adapter is created, used
    /// and dropped there. It may fail, in which case nothing is started.
    pub fn start<S, U, F, E>(cfg: LineConfig, ui: U, make_sip: F) -> Result<Self, E>
    where
        S: SipControl,
        U: Ui + Send + 'static,
        F: FnOnce(EventSink) -> Result<S, E> + Send + 'static,
        E: Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let sink = EventSink(tx);
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), E>>();
        let thread_sink = sink.clone();

        let thread = std::thread::Builder::new()
            .name("softphone-line".into())
            .spawn(move || {
                let sip = match make_sip(thread_sink) {
                    Ok(sip) => {
                        let _ = ready_tx.send(Ok(()));
                        sip
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                run(cfg, sip, ui, rx);
            })
            .expect("spawn line thread");

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self { sink, thread: Some(thread) }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => panic!("line thread exited during start"),
        }
    }

    pub fn sink(&self) -> EventSink {
        self.sink.clone()
    }

    /// Disables the line (hang up, unregister), drops the SIP adapter and
    /// waits for the thread to finish.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = self.sink.0.send(Msg::Shutdown);
            let _ = thread.join();
        }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.stop();
    }
}

type Timers = Rc<RefCell<HashMap<TimerId, Instant>>>;

struct RealClock {
    start: Instant,
    timers: Timers,
}

impl Clock for RealClock {
    fn now(&self) -> Duration {
        self.start.elapsed()
    }
    fn start_timer(&mut self, id: TimerId, after: Duration) {
        self.timers.borrow_mut().insert(id, Instant::now() + after);
    }
    fn cancel_timer(&mut self, id: TimerId) {
        self.timers.borrow_mut().remove(&id);
    }
}

fn run<S: SipControl, U: Ui>(cfg: LineConfig, sip: S, ui: U, rx: Receiver<Msg>) {
    let timers: Timers = Rc::default();
    let clock = RealClock { start: Instant::now(), timers: timers.clone() };
    let mut line = Line::new(cfg, sip, clock, ui);

    loop {
        let next = timers.borrow().values().min().copied();
        let msg = match next {
            Some(at) => rx.recv_timeout(at.saturating_duration_since(Instant::now())),
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match msg {
            Ok(Msg::Event(e)) => line.handle(e),
            Ok(Msg::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                let now = Instant::now();
                let mut due: Vec<(Instant, TimerId)> = timers
                    .borrow()
                    .iter()
                    .filter(|(_, at)| **at <= now)
                    .map(|(id, at)| (*at, *id))
                    .collect();
                due.sort();
                for (_, id) in due {
                    // A handler may have cancelled or restarted it.
                    let still_due = timers.borrow().get(&id).is_some_and(|at| *at <= now);
                    if still_due {
                        timers.borrow_mut().remove(&id);
                        line.handle(Event::Timer(id));
                    }
                }
            }
        }
    }
    line.handle(Event::Disable);
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::ports::{CallId, LineView, Status};

    #[derive(Clone, Default)]
    struct Log(Arc<Mutex<Vec<String>>>);

    impl Log {
        fn push(&self, s: impl Into<String>) {
            self.0.lock().unwrap().push(s.into());
        }
        fn get(&self) -> Vec<String> {
            self.0.lock().unwrap().clone()
        }
    }

    struct Sip(Log);
    impl SipControl for Sip {
        fn register(&mut self) {
            self.0.push("register");
        }
        fn unregister(&mut self) {
            self.0.push("unregister");
        }
        fn answer(&mut self, call: CallId) {
            self.0.push(format!("answer {call}"));
        }
        fn hangup(&mut self, call: CallId) {
            self.0.push(format!("hangup {call}"));
        }
        fn reject_busy(&mut self, call: CallId) {
            self.0.push(format!("busy {call}"));
        }
        fn query_bindings(&mut self) {}
    }
    impl Drop for Sip {
        fn drop(&mut self) {
            self.0.push("dropped");
        }
    }

    struct Statuses(Arc<Mutex<Vec<Status>>>);
    impl Ui for Statuses {
        fn show(&mut self, view: &LineView) {
            self.0.lock().unwrap().push(view.status);
        }
    }

    fn wait_for(log: &Log, entry: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !log.get().iter().any(|e| e == entry) {
            assert!(Instant::now() < deadline, "timed out waiting for {entry}: {:?}", log.get());
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn real_timers_drive_auto_answer_and_shutdown_cleans_up() {
        let log = Log::default();
        let statuses = Arc::new(Mutex::new(vec![]));
        let cfg = LineConfig { auto_answer_delay: Duration::from_millis(20), ..LineConfig::default() };

        let sip_log = log.clone();
        let rt = Runtime::start(cfg, Statuses(statuses.clone()), move |_sink| {
            Ok::<_, ()>(Sip(sip_log))
        })
        .unwrap();

        let sink = rt.sink();
        sink.send(Event::Enable);
        sink.send(Event::Registered);
        sink.send(Event::IncomingCall { call: 3, remote: "<sip:callin@x>".into() });
        wait_for(&log, "answer 3");
        sink.send(Event::CallConfirmed { call: 3 });

        rt.shutdown();
        assert_eq!(log.get(), ["register", "answer 3", "hangup 3", "unregister", "dropped"]);
        assert_eq!(
            *statuses.lock().unwrap(),
            [Status::Disabled, Status::Connecting, Status::Ready, Status::Ringing, Status::OnAir, Status::Disabled]
        );
        assert!(!sink.send(Event::Enable), "sink reports a stopped runtime");
    }

    #[test]
    fn failed_adapter_start_is_reported() {
        let r = Runtime::start(LineConfig::default(), Statuses(Arc::default()), |_sink| {
            Err::<Sip, _>("no TLS")
        });
        assert_eq!(r.err(), Some("no TLS"));
    }
}

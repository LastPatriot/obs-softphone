// SPDX-License-Identifier: GPL-2.0-or-later
//! Checks the media clock without any server: starts PJSIP pointed at a
//! closed local port and counts caller-audio frames for 3 s (expect ~150
//! frames of 960 samples, all silence).
//!   cargo run -p softphone-sip --example media_clock

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use softphone_core::runtime::Runtime;
use softphone_core::{LineConfig, LineView, Ui};
use softphone_sip::{AudioIo, FRAME_SAMPLES, PjsipEngine, SipConfig, Srtp, Transport};

struct NoUi;
impl Ui for NoUi {
    fn show(&mut self, _: &LineView) {}
}

fn main() {
    // Twice: the settings dialog restarts PJSIP in the same process.
    for round in 1..=2 {
        println!("round {round}");
        run_once();
    }
}

fn run_once() {
    let frames = Arc::new(AtomicUsize::new(0));
    let odd = Arc::new(AtomicUsize::new(0));
    let (f, o) = (frames.clone(), odd.clone());
    let cfg = SipConfig {
        server: "127.0.0.1".into(),
        port: 1,
        transport: Transport::Tls,
        username: "101".into(),
        auth_username: "101".into(),
        domain: "127.0.0.1".into(),
        outbound_proxy: String::new(),
        srtp: Srtp::Off,
        stun_server: String::new(),
        password: "x".into(),
        verify_tls: false,
        ca_file: None,
        log_level: 1,
    };
    let rt = Runtime::start(LineConfig::default(), NoUi, move |sink| {
        PjsipEngine::start(
            &cfg,
            sink,
            Box::new(|_, _| {}),
            AudioIo {
                from_caller: Box::new(move |s: &[i16]| {
                    f.fetch_add(1, Ordering::Relaxed);
                    if s.len() != FRAME_SAMPLES || s.iter().any(|&x| x != 0) {
                        o.fetch_add(1, Ordering::Relaxed);
                    }
                }),
                to_caller: Box::new(|out: &mut [i16]| out.fill(0)),
            },
        )
    })
    .expect("start");
    // Let start-up settle, then measure a 3 s window (shutdown may keep
    // the clock running a while longer, so it's not counted).
    std::thread::sleep(Duration::from_millis(500));
    let start = frames.load(Ordering::Relaxed);
    std::thread::sleep(Duration::from_secs(3));
    let n = frames.load(Ordering::Relaxed) - start;
    rt.shutdown();
    println!("frames in 3 s: {n} (expect ~150), non-silent or odd-sized: {}", odd.load(Ordering::Relaxed));
    assert!((140..=160).contains(&n), "media clock not running at 20 ms");
}

// SPDX-License-Identifier: GPL-2.0-or-later
//! Registers a throwaway account over a given transport and prints the line
//! states for a few seconds. Proves a transport works end to end (a 401/403
//! "wrong password" is a pass). Doesn't touch settings or the keychain.
//!   cargo run -p softphone-sip --example transport_check -- <server> <udp|tcp|tls> [port]

use std::time::Duration;

use softphone_core::runtime::Runtime;
use softphone_core::{Event, LineConfig, LineView, Ui};
use softphone_sip::{AudioIo, PjsipEngine, SipConfig, Srtp, Transport};

struct Print;
impl Ui for Print {
    fn show(&mut self, v: &LineView) {
        println!("{:?} {}", v.status, v.message.as_deref().unwrap_or(""));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let server = args.first().expect("server").clone();
    let transport = match args.get(1).map(String::as_str) {
        Some("udp") => Transport::Udp,
        Some("tcp") => Transport::Tcp,
        _ => Transport::Tls,
    };
    let port = args.get(2).and_then(|p| p.parse().ok()).unwrap_or(transport.default_port());
    let cfg = SipConfig {
        server: server.clone(),
        port,
        transport,
        username: "obs-softphone-probe".into(),
        auth_username: "obs-softphone-probe".into(),
        domain: server,
        outbound_proxy: String::new(),
        srtp: Srtp::Off,
        stun_server: String::new(),
        password: "not-a-real-password".into(),
        verify_tls: true,
        ca_file: None,
        log_level: 1,
    };
    let rt = Runtime::start(LineConfig::default(), Print, move |sink| {
        PjsipEngine::start(&cfg, sink, Box::new(|_, _| {}), AudioIo { from_caller: Box::new(|_| {}), to_caller: Box::new(|o| o.fill(0)) })
    })
    .expect("start");
    rt.sink().send(Event::Enable);
    std::thread::sleep(Duration::from_secs(6));
    rt.shutdown();
}

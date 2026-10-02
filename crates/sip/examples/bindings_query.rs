// SPDX-License-Identifier: GPL-2.0-or-later
//! Read-only check of the Replaced-detection query (DESIGN.md §4.4): sends
//! one REGISTER without a Contact header and prints who holds the account.
//! Does not register, so it doesn't disturb a running OBS.
//!   cargo run -p softphone-sip --example bindings_query -- <config.json>

use std::time::Duration;

use softphone_core::runtime::Runtime;
use softphone_core::{LineConfig, LineView, SipControl, Ui};
use softphone_sip::{AudioIo, OsKeyring, PjsipEngine, Settings};

struct NoUi;
impl Ui for NoUi {
    fn show(&mut self, _: &LineView) {}
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: bindings_query <config.json>");
    let (settings, _) = Settings::load(path.as_ref(), &OsKeyring).expect("settings");
    let cfg = settings.sip_config();
    // The runtime is never enabled, so nothing registers.
    let rt = Runtime::start(LineConfig::default(), NoUi, move |sink| {
        let mut engine = PjsipEngine::start(
            &cfg,
            sink,
            Box::new(|level, msg| {
                if level <= 3 {
                    println!("pj{level} {msg}");
                }
            }),
            AudioIo { from_caller: Box::new(|_| {}), to_caller: Box::new(|o| o.fill(0)) },
        )?;
        engine.query_bindings();
        Ok::<_, String>(engine)
    })
    .expect("start");
    std::thread::sleep(Duration::from_secs(5));
    rt.shutdown();
}

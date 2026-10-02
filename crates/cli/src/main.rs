// SPDX-License-Identifier: GPL-2.0-or-later
//! `softphone-cli <config.json> [--tone]`: registers the SIP account and
//! auto-answers, printing every state change and the caller's level.
//! `--tone` sends the caller a 440 Hz tone (tests the return feed path).
//! Commands on stdin:
//!   a answer · e end call · r retry · t take the line back
//!   on / off enable / disable · auto on|off · q quit

use std::io::BufRead;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use softphone_core::runtime::Runtime;
use softphone_core::{Event, LineView, Ui};
use softphone_sip::{AudioIo, OsKeyring, PjsipEngine, SAMPLE_RATE, Settings};

struct ConsoleUi(Instant);

impl Ui for ConsoleUi {
    fn show(&mut self, v: &LineView) {
        let mut line = format!("[{:>8.1}s] {:?}", self.0.elapsed().as_secs_f32(), v.status);
        if let Some(caller) = &v.caller {
            line += &format!(" · {caller}");
        }
        if let Some(msg) = &v.message {
            line += &format!(" · {msg}");
        }
        println!("{line}");
    }
}

/// Prints the caller's level once a second while there is sound, so the
/// audio path can be checked without OBS.
fn level_meter() -> impl Fn(&[i16]) + Send + Sync {
    struct Acc {
        sum_sq: f64,
        n: usize,
        peak: i32,
        since: Instant,
    }
    let acc = Mutex::new(Acc { sum_sq: 0.0, n: 0, peak: 0, since: Instant::now() });
    move |samples| {
        let mut a = acc.lock().unwrap();
        for &s in samples {
            a.sum_sq += f64::from(s) * f64::from(s);
            a.peak = a.peak.max(i32::from(s).abs());
        }
        a.n += samples.len();
        if a.since.elapsed() >= Duration::from_secs(1) {
            if a.peak > 0 {
                let rms = (a.sum_sq / a.n as f64).sqrt() / 32768.0;
                let peak = f64::from(a.peak) / 32768.0;
                println!(
                    "  caller level: rms {:6.1} dBFS, peak {:6.1} dBFS ({} samples)",
                    20.0 * rms.max(1e-9).log10(),
                    20.0 * peak.log10(),
                    a.n
                );
            }
            *a = Acc { sum_sq: 0.0, n: 0, peak: 0, since: Instant::now() };
        }
    }
}

/// What the caller hears: a 440 Hz tone at -20 dBFS, or silence.
fn return_audio(tone: bool) -> impl Fn(&mut [i16]) + Send + Sync {
    let phase = Mutex::new(0u64);
    move |out| {
        if !tone {
            out.fill(0);
            return;
        }
        let mut n = phase.lock().unwrap();
        for o in out.iter_mut() {
            let t = *n as f64 / f64::from(SAMPLE_RATE);
            *o = (3277.0 * (2.0 * std::f64::consts::PI * 440.0 * t).sin()) as i16;
            *n += 1;
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let tone = args.iter().any(|a| a == "--tone");
    let Some(path) = args.iter().find(|a| !a.starts_with("--")).map(PathBuf::from) else {
        eprintln!("usage: softphone-cli <config.json> [--tone]   (config created with defaults if missing)");
        std::process::exit(2);
    };
    // On macOS the Keychain may ask once to let this tool read the
    // password the OBS plugin stored.
    let settings = match Settings::load(&path, &OsKeyring) {
        Ok((s, notes)) => {
            notes.iter().for_each(|n| eprintln!("settings: {n}"));
            s
        }
        Err(e) => {
            eprintln!("settings: {e}");
            std::process::exit(1);
        }
    };
    if settings.server.is_empty() || settings.password.is_empty() {
        eprintln!("Fill in \"server\" and \"password\" in {}", path.display());
        std::process::exit(1);
    }

    let sip_cfg = settings.sip_config();
    let runtime = Runtime::start(settings.line_config(), ConsoleUi(Instant::now()), move |sink| {
        PjsipEngine::start(
            &sip_cfg,
            sink,
            Box::new(|level, msg| eprintln!("  pj{level} {msg}")),
            AudioIo { from_caller: Box::new(level_meter()), to_caller: Box::new(return_audio(tone)) },
        )
    })
    .unwrap_or_else(|e| {
        eprintln!("SIP start failed: {e}");
        std::process::exit(1);
    });

    let sink = runtime.sink();
    if settings.enabled {
        sink.send(Event::Enable);
    }

    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let event = match line.trim() {
            "a" => Event::Answer,
            "e" => Event::EndCall,
            "r" => Event::Retry,
            "t" => Event::TakeBack,
            "on" => Event::Enable,
            "off" => Event::Disable,
            "auto on" => Event::SetAutoAnswer(true),
            "auto off" => Event::SetAutoAnswer(false),
            "q" => break,
            "" => continue,
            other => {
                eprintln!("unknown command {other:?} (a, e, r, t, on, off, auto on|off, q)");
                continue;
            }
        };
        sink.send(event);
    }

    runtime.shutdown();
}

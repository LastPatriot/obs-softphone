// SPDX-License-Identifier: GPL-2.0-or-later
//! PJSIP adapter (DESIGN.md §3.1): implements the core's [`SipControl`] and
//! turns pjsua callbacks into core [`Event`]s.

mod ffi;
mod return_feed;
mod settings;

use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_void};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use softphone_core::runtime::EventSink;
use softphone_core::{Binding, CallId, Event, SipControl};

pub use return_feed::{FeedStats, ReturnFeed};
pub use settings::{OsKeyring, Preset, SecretStore, Settings, Srtp, Transport};

#[derive(Clone, Debug)]
pub struct SipConfig {
    /// Registrar host (and where requests go, unless `outbound_proxy`).
    pub server: String,
    pub port: u16,
    pub transport: Transport,
    pub username: String,
    pub auth_username: String,
    pub domain: String,
    pub outbound_proxy: String,
    pub srtp: Srtp,
    pub stun_server: String,
    pub password: String,
    pub verify_tls: bool,
    pub ca_file: Option<PathBuf>,
    /// pjlib log level, 0..=6 (5+ includes full SIP messages).
    pub log_level: i32,
}

/// pjlib levels: 1 error, 2 warning, 3 info, 4+ debug.
pub type LogFn = Box<dyn Fn(i32, &str) + Send + Sync>;

/// Receives the caller's audio: [`FRAME_SAMPLES`] of 48 kHz mono every
/// 20 ms, silence when there is no call. Runs on PJSIP's media clock
/// thread, so it must return quickly and never block for long.
pub type AudioFn = Box<dyn Fn(&[i16]) + Send + Sync>;

/// Fills what the caller hears, 20 ms at a time, during a call. Same
/// thread rules as [`AudioFn`].
pub type ReturnFn = Box<dyn Fn(&mut [i16]) + Send + Sync>;

/// The two audio directions between the call and the host application.
pub struct AudioIo {
    pub from_caller: AudioFn,
    pub to_caller: ReturnFn,
}

pub const SAMPLE_RATE: u32 = 48_000;
pub const FRAME_SAMPLES: usize = 960;

struct Ctx {
    sink: EventSink,
    log: LogFn,
    audio: AudioIo,
}

static RUNNING: AtomicBool = AtomicBool::new(false);

/// The one pjsua instance in this process. Dropping it hangs up, unregisters
/// and shuts pjsua down.
pub struct PjsipEngine {
    ctx: *mut Ctx,
}

// The shim registers whichever thread calls it with pjlib.
unsafe impl Send for PjsipEngine {}

impl PjsipEngine {
    pub fn start(cfg: &SipConfig, sink: EventSink, log: LogFn, audio: AudioIo) -> Result<Self, String> {
        if cfg.server.trim().is_empty() {
            return Err("No SIP server configured".into());
        }
        if RUNNING.swap(true, Ordering::SeqCst) {
            return Err("PJSIP is already running in this process".into());
        }

        let cstr = |s: &str| CString::new(s).map_err(|_| format!("setting contains a NUL byte: {s:?}"));
        let strings = (|| {
            Ok::<_, String>([
                cstr(&cfg.server)?,
                cstr(&cfg.username)?,
                cstr(&cfg.auth_username)?,
                cstr(&cfg.domain)?,
                cstr(&cfg.outbound_proxy)?,
                cstr(&cfg.stun_server)?,
                cstr(&cfg.password)?,
            ])
        })();
        let [server, username, auth_username, domain, outbound_proxy, stun_server, password] = match strings {
            Ok(s) => s,
            Err(e) => {
                RUNNING.store(false, Ordering::SeqCst);
                return Err(e);
            }
        };
        let ca_file = cfg
            .ca_file
            .clone()
            .or_else(default_ca_file)
            .and_then(|p| CString::new(p.to_string_lossy().into_owned()).ok());

        let ctx = Box::into_raw(Box::new(Ctx { sink, log, audio }));
        let callbacks = ffi::SpCallbacks {
            ud: ctx.cast(),
            on_reg,
            on_incoming,
            on_call_state,
            on_transport_down,
            on_options,
            on_bindings,
            on_log,
            on_caller_audio,
            on_return_audio,
        };
        let sp_cfg = ffi::SpConfig {
            server: server.as_ptr(),
            port: c_int::from(cfg.port),
            transport: match cfg.transport {
                Transport::Udp => 0,
                Transport::Tcp => 1,
                Transport::Tls => 2,
            },
            username: username.as_ptr(),
            auth_username: auth_username.as_ptr(),
            domain: domain.as_ptr(),
            outbound_proxy: outbound_proxy.as_ptr(),
            stun_server: stun_server.as_ptr(),
            srtp: match cfg.srtp {
                Srtp::Off => 0,
                Srtp::Optional => 1,
                Srtp::Required => 2,
            },
            password: password.as_ptr(),
            verify_tls: c_int::from(cfg.verify_tls),
            ca_file: ca_file.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
            log_level: cfg.log_level.clamp(0, 6),
        };

        let mut err = [0 as c_char; 256];
        // SAFETY: all pointers are valid for the call; the shim copies the
        // strings it keeps. `ctx` stays alive until `sp_destroy` returns.
        let rc = unsafe { ffi::sp_init(&sp_cfg, &callbacks, err.as_mut_ptr(), err.len() as c_int) };
        if rc != 0 {
            // SAFETY: sp_init failed and has already torn pjsua down.
            drop(unsafe { Box::from_raw(ctx) });
            RUNNING.store(false, Ordering::SeqCst);
            // SAFETY: the shim NUL-terminates `err` (snprintf).
            let msg = unsafe { CStr::from_ptr(err.as_ptr()) }.to_string_lossy().into_owned();
            return Err(msg);
        }
        Ok(Self { ctx })
    }
}

impl Drop for PjsipEngine {
    fn drop(&mut self) {
        // SAFETY: after sp_destroy returns no callback can use `ctx`.
        unsafe {
            ffi::sp_destroy();
            drop(Box::from_raw(self.ctx));
        }
        RUNNING.store(false, Ordering::SeqCst);
    }
}

impl SipControl for PjsipEngine {
    fn register(&mut self) {
        unsafe { ffi::sp_register() }
    }
    fn unregister(&mut self) {
        unsafe { ffi::sp_unregister() }
    }
    fn answer(&mut self, call: CallId) {
        unsafe { ffi::sp_answer(call) }
    }
    fn hangup(&mut self, call: CallId) {
        unsafe { ffi::sp_hangup(call) }
    }
    fn reject_busy(&mut self, call: CallId) {
        unsafe { ffi::sp_reject_busy(call) }
    }
    fn query_bindings(&mut self) {
        unsafe { ffi::sp_query_bindings() }
    }
}

/// The CA bundle to give pjlib's TLS when the settings name none.
/// macOS uses Network.framework, which trusts the system store by itself
/// (and expects DER, not a PEM bundle), so nothing is passed there. Other
/// platforms' OpenSSL/GnuTLS load no CAs by default, so pass the system's.
fn default_ca_file() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        return None;
    }
    [
        "/etc/ssl/certs/ca-certificates.crt", // Debian/Ubuntu
        "/etc/pki/tls/certs/ca-bundle.crt",   // Fedora
        "/etc/ssl/cert.pem",                  // Alpine, BSDs
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|p| p.is_file())
}

// --- callbacks (pjsua worker thread) -----------------------------------------

/// SAFETY: `ud` is the `Ctx` from `start`, alive until `sp_destroy` returns.
unsafe fn ctx<'a>(ud: *mut c_void) -> &'a Ctx {
    unsafe { &*ud.cast::<Ctx>() }
}

/// SAFETY: `s` is NULL or a valid NUL-terminated string.
unsafe fn string(s: *const c_char) -> String {
    if s.is_null() { String::new() } else { unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned() }
}

extern "C" fn on_reg(ud: *mut c_void, code: c_int, reason: *const c_char, renew: c_int) {
    let (ctx, reason) = unsafe { (ctx(ud), string(reason)) };
    let ok = (200..300).contains(&code);
    let event = match (renew != 0, ok) {
        (true, true) => Event::Registered,
        (false, true) => Event::Unregistered,
        (_, false) => Event::RegistrationFailed { code: code.clamp(0, 999) as u16, reason },
    };
    ctx.sink.send(event);
}

extern "C" fn on_incoming(ud: *mut c_void, call: c_int, remote: *const c_char) {
    let (ctx, remote) = unsafe { (ctx(ud), string(remote)) };
    ctx.sink.send(Event::IncomingCall { call, remote });
}

extern "C" fn on_call_state(ud: *mut c_void, call: c_int, state: c_int) {
    let ctx = unsafe { ctx(ud) };
    match state {
        ffi::SP_CALL_CONFIRMED => ctx.sink.send(Event::CallConfirmed { call }),
        ffi::SP_CALL_DISCONNECTED => ctx.sink.send(Event::CallEnded { call }),
        _ => true,
    };
}

extern "C" fn on_transport_down(ud: *mut c_void) {
    unsafe { ctx(ud) }.sink.send(Event::TransportDown);
}

extern "C" fn on_options(ud: *mut c_void) {
    unsafe { ctx(ud) }.sink.send(Event::OptionsReceived);
}

extern "C" fn on_bindings(ud: *mut c_void, binding: c_int) {
    let binding = match binding {
        ffi::SP_BINDING_OURS => Binding::Ours,
        ffi::SP_BINDING_OTHER => Binding::Other,
        ffi::SP_BINDING_NONE => Binding::None,
        _ => Binding::Unknown,
    };
    unsafe { ctx(ud) }.sink.send(Event::Bindings(binding));
}

extern "C" fn on_log(ud: *mut c_void, level: c_int, msg: *const c_char, len: c_int) {
    if msg.is_null() || len <= 0 {
        return;
    }
    // SAFETY: pjlib passes `len` valid bytes.
    let bytes = unsafe { std::slice::from_raw_parts(msg.cast::<u8>(), len as usize) };
    let text = String::from_utf8_lossy(bytes);
    (unsafe { ctx(ud) }.log)(level, text.trim_end());
}

extern "C" fn on_caller_audio(ud: *mut c_void, samples: *const i16, count: c_uint) {
    if samples.is_null() || count == 0 {
        return;
    }
    // SAFETY: the shim passes `count` valid samples for the duration of the call.
    let samples = unsafe { std::slice::from_raw_parts(samples, count as usize) };
    (unsafe { ctx(ud) }.audio.from_caller)(samples);
}

extern "C" fn on_return_audio(ud: *mut c_void, out: *mut i16, count: c_uint) {
    if out.is_null() || count == 0 {
        return;
    }
    // SAFETY: the shim passes a writable buffer of `count` samples.
    let out = unsafe { std::slice::from_raw_parts_mut(out, count as usize) };
    (unsafe { ctx(ud) }.audio.to_caller)(out);
}

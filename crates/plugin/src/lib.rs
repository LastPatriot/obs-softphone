// SPDX-License-Identifier: GPL-2.0-or-later
//! OBS module entry points (the C ABI that OBS_DECLARE_MODULE would generate).
//!
//! Registers as the studio line, auto-answers, shows the state in the
//! Call-In dock (M1) and outputs the caller as the "Call-In Caller" audio
//! source (M2), sends the chosen OBS track back to the caller (M3), with
//! the settings dialog, warnings and mute in the dock (M4).

mod caller_source;
mod checks;
mod dock;
mod line;
mod obs;
mod return_tap;

use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicPtr, Ordering};

static MODULE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

#[unsafe(no_mangle)]
pub extern "C" fn obs_module_set_pointer(module: *mut c_void) {
    MODULE.store(module, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
pub extern "C" fn obs_current_module() -> *mut c_void {
    MODULE.load(Ordering::SeqCst)
}

#[unsafe(no_mangle)]
pub extern "C" fn obs_module_ver() -> u32 {
    obs::LIBOBS_API_VER
}

#[unsafe(no_mangle)]
pub extern "C" fn obs_module_name() -> *const c_char {
    c"SIP Call-In".as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn obs_module_description() -> *const c_char {
    c"Receive SIP calls in OBS as an audio source, with mix-minus".as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn obs_module_load() -> bool {
    obs::log(obs::LOG_INFO, concat!("loading version ", env!("CARGO_PKG_VERSION")));
    // Registered even when the line isn't configured, so scenes keep it.
    caller_source::register();

    let widget = dock::create();
    if !obs::add_dock(c"sip_callin_dock", c"Call-In", widget) {
        obs::log(obs::LOG_WARNING, "could not add the Call-In dock");
    }
    obs::add_tools_menu_item(c"SIP Call-In…", on_tools_menu);

    match obs::config_path(obs_current_module(), c"config.json") {
        Some(path) => line::init(path),
        None => dock::show_setup("OBS gave no config directory for the plugin."),
    }
    true
}

#[unsafe(no_mangle)]
pub extern "C" fn obs_module_unload() {
    dock::begin_shutdown();
    line::shutdown();
    dock::finish_shutdown();
    obs::log(obs::LOG_INFO, "unloaded");
}

extern "C" fn on_tools_menu(_data: *mut c_void) {
    line::open_settings();
}

// SPDX-License-Identifier: GPL-2.0-or-later
//! The few libobs / obs-frontend-api functions the plugin uses, declared by hand.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::PathBuf;

pub type ObsModule = c_void;

pub const LOG_ERROR: c_int = 100;
pub const LOG_WARNING: c_int = 200;
pub const LOG_INFO: c_int = 300;
pub const LOG_DEBUG: c_int = 400;

/// MAKE_SEMANTIC_VERSION(30, 0, 0): obs_frontend_add_dock_by_id needs OBS 30.
pub const LIBOBS_API_VER: u32 = 30 << 24;

unsafe extern "C" {
    fn blog(log_level: c_int, format: *const c_char, ...);
    fn bfree(ptr: *mut c_void);
    fn obs_module_get_config_path(module: *mut ObsModule, file: *const c_char) -> *mut c_char;
    fn obs_frontend_add_dock_by_id(id: *const c_char, title: *const c_char, widget: *mut c_void) -> bool;
    fn obs_frontend_get_main_window() -> *mut c_void;
    fn obs_frontend_add_tools_menu_item(name: *const c_char, cb: extern "C" fn(*mut c_void), data: *mut c_void);
}

pub fn log(level: c_int, msg: &str) {
    let msg = CString::new(format!("[obs-softphone] {msg}").replace('\0', " ")).unwrap();
    // SAFETY: "%s" with one valid C string.
    unsafe { blog(level, c"%s".as_ptr(), msg.as_ptr()) };
}

/// `<OBS config>/plugin_config/obs-softphone/<file>`
pub fn config_path(module: *mut ObsModule, file: &CStr) -> Option<PathBuf> {
    // SAFETY: module is the pointer OBS gave us; result is bmalloc'd or NULL.
    unsafe {
        let p = obs_module_get_config_path(module, file.as_ptr());
        if p.is_null() {
            return None;
        }
        let path = PathBuf::from(CStr::from_ptr(p).to_string_lossy().into_owned());
        bfree(p.cast());
        Some(path)
    }
}

/// Must be called on the UI thread. OBS takes ownership of `widget`.
pub fn add_dock(id: &CStr, title: &CStr, widget: *mut c_void) -> bool {
    unsafe { obs_frontend_add_dock_by_id(id.as_ptr(), title.as_ptr(), widget) }
}

/// The OBS main window (a QMainWindow*), for parenting dialogs.
pub fn main_window() -> *mut c_void {
    unsafe { obs_frontend_get_main_window() }
}

/// UI thread. `cb` runs on the UI thread when the item is chosen.
pub fn add_tools_menu_item(name: &'static CStr, cb: extern "C" fn(*mut c_void)) {
    unsafe { obs_frontend_add_tools_menu_item(name.as_ptr(), cb, std::ptr::null_mut()) };
}

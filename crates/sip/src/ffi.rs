// SPDX-License-Identifier: GPL-2.0-or-later
//! Declarations for shim/sp_shim.h.

use std::ffi::{c_char, c_int, c_uint, c_void};

pub const SP_CALL_CONFIRMED: c_int = 1;
pub const SP_CALL_DISCONNECTED: c_int = 2;

pub const SP_BINDING_OURS: c_int = 0;
pub const SP_BINDING_OTHER: c_int = 1;
pub const SP_BINDING_NONE: c_int = 2;

#[repr(C)]
pub struct SpCallbacks {
    pub ud: *mut c_void,
    pub on_reg: extern "C" fn(*mut c_void, c_int, *const c_char, c_int),
    pub on_incoming: extern "C" fn(*mut c_void, c_int, *const c_char),
    pub on_call_state: extern "C" fn(*mut c_void, c_int, c_int),
    pub on_transport_down: extern "C" fn(*mut c_void),
    pub on_options: extern "C" fn(*mut c_void),
    pub on_bindings: extern "C" fn(*mut c_void, c_int),
    pub on_log: extern "C" fn(*mut c_void, c_int, *const c_char, c_int),
    pub on_caller_audio: extern "C" fn(*mut c_void, *const i16, c_uint),
    pub on_return_audio: extern "C" fn(*mut c_void, *mut i16, c_uint),
}

#[repr(C)]
pub struct SpConfig {
    pub server: *const c_char,
    pub port: c_int,
    pub transport: c_int,
    pub username: *const c_char,
    pub auth_username: *const c_char,
    pub domain: *const c_char,
    pub outbound_proxy: *const c_char,
    pub stun_server: *const c_char,
    pub srtp: c_int,
    pub password: *const c_char,
    pub verify_tls: c_int,
    pub ca_file: *const c_char,
    pub log_level: c_int,
}

unsafe extern "C" {
    pub fn sp_init(cfg: *const SpConfig, cb: *const SpCallbacks, err: *mut c_char, err_len: c_int) -> c_int;
    pub fn sp_register();
    pub fn sp_unregister();
    pub fn sp_answer(call_id: c_int);
    pub fn sp_hangup(call_id: c_int);
    pub fn sp_reject_busy(call_id: c_int);
    pub fn sp_query_bindings();
    pub fn sp_destroy();
}

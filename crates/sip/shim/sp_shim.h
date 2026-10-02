// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * Narrow C API over pjsua for the Rust side (crates/sip/src/ffi.rs).
 * Keeps pjsua's large, layout-sensitive config structs on the C side.
 * One instance per process.
 */
#ifndef SP_SHIM_H
#define SP_SHIM_H

#ifdef __cplusplus
extern "C" {
#endif

enum sp_call_state { SP_CALL_CONFIRMED = 1, SP_CALL_DISCONNECTED = 2 };
enum sp_binding { SP_BINDING_OURS = 0, SP_BINDING_OTHER = 1, SP_BINDING_NONE = 2, SP_BINDING_UNKNOWN = 3 };

typedef struct sp_callbacks {
	void *ud;
	/* renew: 1 for a (re-)registration result, 0 for an unregistration result. */
	void (*on_reg)(void *ud, int code, const char *reason, int renew);
	void (*on_incoming)(void *ud, int call_id, const char *remote);
	void (*on_call_state)(void *ud, int call_id, int state);
	void (*on_transport_down)(void *ud);
	void (*on_options)(void *ud);
	void (*on_bindings)(void *ud, int binding);
	void (*on_log)(void *ud, int level, const char *msg, int len);
	/* The caller's audio: SP_FRAME_SAMPLES of 48 kHz mono every 20 ms, on
	 * PJSIP's media clock thread; silence when there is no call. Must not
	 * block. */
	void (*on_caller_audio)(void *ud, const short *samples, unsigned count);
	/* What the caller hears: fill `count` samples (48 kHz mono) every 20 ms
	 * during a call, on the media clock thread. Must not block. */
	void (*on_return_audio)(void *ud, short *out, unsigned count);
} sp_callbacks;

#define SP_SAMPLE_RATE 48000
#define SP_FRAME_SAMPLES 960 /* 20 ms */

enum sp_transport { SP_TRANSPORT_UDP = 0, SP_TRANSPORT_TCP = 1, SP_TRANSPORT_TLS = 2 };
enum sp_srtp { SP_SRTP_OFF = 0, SP_SRTP_OPTIONAL = 1, SP_SRTP_REQUIRED = 2 };

typedef struct sp_config {
	const char *server; /* registrar host */
	int port;
	int transport;      /* enum sp_transport */
	const char *username;
	const char *auth_username;  /* digest user */
	const char *domain;         /* AOR domain: sip:username@domain */
	const char *outbound_proxy; /* "" or host[:port] */
	const char *stun_server;    /* "" or host[:port] */
	int srtp;                   /* enum sp_srtp */
	const char *password;
	int verify_tls;
	const char *ca_file; /* may be NULL */
	int log_level;       /* pjlib level 0..6 */
} sp_config;

/* Returns 0 on success; on failure writes a message to err. */
int sp_init(const sp_config *cfg, const sp_callbacks *cb, char *err, int err_len);
void sp_register(void);
void sp_unregister(void);
void sp_answer(int call_id);
void sp_hangup(int call_id);
void sp_reject_busy(int call_id);
void sp_query_bindings(void);
/* Hangs up, unregisters and shuts pjsua down. Callbacks may fire until it returns. */
void sp_destroy(void);

#ifdef __cplusplus
}
#endif

#endif

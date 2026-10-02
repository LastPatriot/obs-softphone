// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * pjsua glue for the OBS softphone (DESIGN.md §3.1). See sp_shim.h.
 *
 * Threads: pjsua callbacks run on pjsua's worker thread; sp_* calls come from
 * the Rust line thread. Every entry point registers its thread with pjlib.
 */
#include "sp_shim.h"

#include <pjsua-lib/pjsua.h>
#include <stdio.h>
#include <string.h>

#define THIS_FILE "sp_shim.c"

static sp_callbacks g_cb;
static pjsua_acc_id g_acc = PJSUA_INVALID_ID;
static pj_pool_t *g_pool;

/* For the bindings query (§4.4). */
static pjsip_regc *g_query;
static pj_pool_t *g_binding_pool; /* reset on each successful registration */
static pjsip_uri *g_binding;      /* our binding as the server reported it */
static pj_mutex_t *g_binding_lock;

/* Set while an unregistration we started is in flight (see sp_destroy). */
static volatile int g_unreg_pending;

static char g_srv_uri[300];
static char g_aor[300];
static char g_username[128]; /* the digest user */
static char g_password[128];
static char g_proxy[300];
static char g_stun[256];
static const char *g_transport_param = "tls";

#if defined(_MSC_VER)
#define SP_THREAD_LOCAL __declspec(thread)
#else
#define SP_THREAD_LOCAL __thread
#endif

static void ensure_thread(void)
{
	static SP_THREAD_LOCAL pj_thread_desc desc;
	pj_thread_t *thread;

	if (!pj_thread_is_registered())
		pj_thread_register("sp-ext", desc, &thread);
}

static void set_err(char *err, int len, const char *what, pj_status_t status)
{
	char msg[PJ_ERR_MSG_SIZE];

	pj_strerror(status, msg, sizeof(msg));
	snprintf(err, (size_t)len, "%s: %s", what, msg);
}

/* --- logging ------------------------------------------------------------ */

static void on_log(int level, const char *data, int len)
{
	if (g_cb.on_log)
		g_cb.on_log(g_cb.ud, level, data, len);
}

/* --- registration --------------------------------------------------------- */

/* Remember the binding the server reports for us (it may have rewritten our
 * Contact, so our own Contact can't be compared directly). With
 * max_contacts=1 there is exactly one. */
static void remember_binding(const struct pjsip_regc_cbparam *p)
{
	char buf[512];
	int n;
	pjsip_uri *uri;

	pj_mutex_lock(g_binding_lock);
	g_binding = NULL;
	pj_pool_reset(g_binding_pool);
	if (p->contact_cnt == 1) {
		n = pjsip_uri_print(PJSIP_URI_IN_CONTACT_HDR,
				    pjsip_uri_get_uri(p->contact[0]->uri), buf, sizeof(buf) - 1);
		if (n > 0) {
			buf[n] = '\0';
			uri = pjsip_parse_uri(g_binding_pool, buf, (pj_size_t)n, 0);
			g_binding = uri;
		}
	}
	pj_mutex_unlock(g_binding_lock);
}

static void on_reg_state2(pjsua_acc_id acc_id, pjsua_reg_info *info)
{
	const struct pjsip_regc_cbparam *p = info->cbparam;
	char reason[128];
	int code;

	PJ_UNUSED_ARG(acc_id);
	if (!p)
		return;

	code = p->code;
	if (p->status != PJ_SUCCESS && code / 100 == 2)
		code = 503;
	if (code == 0)
		code = 503; /* transport/TLS failure before any response */

	snprintf(reason, sizeof(reason), "%.*s", (int)p->reason.slen, p->reason.ptr);
	if (code == 503 && p->status != PJ_SUCCESS) {
		char msg[PJ_ERR_MSG_SIZE];
		pj_strerror(p->status, msg, sizeof(msg));
		snprintf(reason, sizeof(reason), "%s", msg);
	}

	if (info->renew && code / 100 == 2)
		remember_binding(p);
	if (!info->renew)
		g_unreg_pending = 0;

	if (g_cb.on_reg)
		g_cb.on_reg(g_cb.ud, code, reason, info->renew ? 1 : 0);
}

/* --- bindings query: REGISTER without Contact (RFC 3261 §10.2.4) ---------- */

static void on_query_response(struct pjsip_regc_cbparam *p)
{
	int result = SP_BINDING_UNKNOWN;
	int i;

	if (p->status == PJ_SUCCESS && p->code / 100 == 2) {
		if (p->contact_cnt == 0) {
			result = SP_BINDING_NONE;
		} else {
			pj_mutex_lock(g_binding_lock);
			if (g_binding) {
				result = SP_BINDING_OTHER;
				for (i = 0; i < p->contact_cnt; ++i) {
					if (pjsip_uri_cmp(PJSIP_URI_IN_CONTACT_HDR, g_binding,
							  pjsip_uri_get_uri(p->contact[i]->uri)) == PJ_SUCCESS) {
						result = SP_BINDING_OURS;
						break;
					}
				}
			}
			pj_mutex_unlock(g_binding_lock);
		}
	}
	PJ_LOG(3, (THIS_FILE, "Bindings query: code=%d contacts=%d result=%d", p->code, p->contact_cnt, result));
	for (i = 0; i < p->contact_cnt; ++i) {
		char buf[256];
		int n = pjsip_uri_print(PJSIP_URI_IN_CONTACT_HDR, pjsip_uri_get_uri(p->contact[i]->uri), buf,
					sizeof(buf) - 1);
		if (n > 0)
			PJ_LOG(3, (THIS_FILE, "  bound: %.*s", n, buf));
	}
	if (g_cb.on_bindings)
		g_cb.on_bindings(g_cb.ud, result);
}

void sp_query_bindings(void)
{
	pjsip_tx_data *tdata;
	pj_str_t srv, aor;
	pjsip_cred_info cred;
	pj_status_t status;

	ensure_thread();

	if (!g_query) {
		status = pjsip_regc_create(pjsua_get_pjsip_endpt(), NULL, &on_query_response, &g_query);
		if (status == PJ_SUCCESS) {
			srv = pj_str(g_srv_uri);
			aor = pj_str(g_aor);
			/* No contacts and no Expires header: a pure query. */
			status = pjsip_regc_init(g_query, &srv, &aor, &aor, 0, NULL,
						 PJSIP_REGC_EXPIRATION_NOT_SPECIFIED);
		}
		if (status == PJ_SUCCESS) {
			pj_bzero(&cred, sizeof(cred));
			cred.realm = pj_str("*");
			cred.scheme = pj_str("digest");
			cred.username = pj_str(g_username);
			cred.data_type = PJSIP_CRED_DATA_PLAIN_PASSWD;
			cred.data = pj_str(g_password);
			status = pjsip_regc_set_credentials(g_query, 1, &cred);
		}
		if (status == PJ_SUCCESS && g_proxy[0]) {
			pjsip_route_hdr route_set, *route;
			pj_str_t hname = {"Route", 5};
			char value[320];
			int len = snprintf(value, sizeof(value), "<%s>", g_proxy);
			pj_list_init(&route_set);
			route = (pjsip_route_hdr *)pjsip_parse_hdr(g_pool, &hname, value, (pj_size_t)len, NULL);
			if (route) {
				pj_list_push_back(&route_set, route);
				status = pjsip_regc_set_route_set(g_query, &route_set);
			}
		}
		if (status != PJ_SUCCESS) {
			if (g_query) {
				pjsip_regc_destroy(g_query);
				g_query = NULL;
			}
			PJ_PERROR(2, (THIS_FILE, status, "Can't set up bindings query"));
			if (g_cb.on_bindings)
				g_cb.on_bindings(g_cb.ud, SP_BINDING_UNKNOWN);
			return;
		}
	}

	status = pjsip_regc_register(g_query, PJ_FALSE, &tdata);
	if (status == PJ_SUCCESS)
		status = pjsip_regc_send(g_query, tdata);
	if (status != PJ_SUCCESS) {
		PJ_PERROR(2, (THIS_FILE, status, "Bindings query failed to send"));
		if (g_cb.on_bindings)
			g_cb.on_bindings(g_cb.ud, SP_BINDING_UNKNOWN);
	}
}

/* --- OPTIONS sniffer (§4.4) ----------------------------------------------- */

/* Sees every incoming request before pjsua handles it, and never consumes it:
 * counts OPTIONS for the Replaced watchdog, and logs new INVITEs (pjsua
 * only logs them at level 4). */
static pj_bool_t on_rx_request(pjsip_rx_data *rdata)
{
	pjsip_method_e method = rdata->msg_info.msg->line.req.method.id;

	if (method == PJSIP_OPTIONS_METHOD && g_cb.on_options)
		g_cb.on_options(g_cb.ud);
	if (method == PJSIP_INVITE_METHOD && rdata->msg_info.to->tag.slen == 0)
		PJ_LOG(3, (THIS_FILE, "Incoming INVITE from %.*s", (int)rdata->msg_info.from->name.slen,
			   rdata->msg_info.from->name.ptr ? rdata->msg_info.from->name.ptr : ""));
	return PJ_FALSE;
}

/* pjsua rejects some INVITEs (e.g. 488, no common codec) without logging
 * anything below level 4; make every rejection visible. */
static pj_status_t on_tx_response(pjsip_tx_data *tdata)
{
	const pjsip_cseq_hdr *cseq = PJSIP_MSG_CSEQ_HDR(tdata->msg);
	int code = tdata->msg->line.status.code;

	const pj_str_t *reason = &tdata->msg->line.status.reason;

	if (!cseq || cseq->method.id != PJSIP_INVITE_METHOD || code < 300)
		return PJ_SUCCESS;
	/* PJ_LOG needs a constant level. 486/487 are our own busy/cancel. */
	if (code == 486 || code == 487)
		PJ_LOG(3, (THIS_FILE, "Incoming call answered with %d %.*s", code, (int)reason->slen, reason->ptr));
	else
		PJ_LOG(2, (THIS_FILE, "Incoming call rejected with %d %.*s", code, (int)reason->slen, reason->ptr));
	return PJ_SUCCESS;
}

static pjsip_module g_options_sniffer = {
	.name = {"mod-sp-sniffer", 14},
	.id = -1,
	.priority = PJSIP_MOD_PRIORITY_TSX_LAYER - 1,
	.on_rx_request = &on_rx_request,
	.on_tx_response = &on_tx_response,
};

/* --- transport --------------------------------------------------------------- */

static void on_transport_state(pjsip_transport *tp, pjsip_transport_state state,
			       const pjsip_transport_state_info *info)
{
	PJ_UNUSED_ARG(info);
	/* Connection-oriented transports only; UDP has no connection to lose. */
	if (state == PJSIP_TP_STATE_DISCONNECTED &&
	    (tp->key.type == PJSIP_TRANSPORT_TLS || tp->key.type == PJSIP_TRANSPORT_TCP) && g_cb.on_transport_down)
		g_cb.on_transport_down(g_cb.ud);
}

/* --- media port between the call and OBS (§3.1) ----------------------------- */

static pjmedia_port g_obs_port;
static pjsua_conf_port_id g_obs_slot = PJSUA_INVALID_ID;
static const pj_int16_t g_silence[SP_FRAME_SAMPLES];

/* Conference -> us: the caller's audio. Called every 20 ms on the media
 * clock thread; a NONE frame (nobody transmitting) means silence. */
static pj_status_t obs_put_frame(pjmedia_port *port, pjmedia_frame *frame)
{
	const pj_int16_t *samples = g_silence;
	unsigned count = SP_FRAME_SAMPLES;

	PJ_UNUSED_ARG(port);
	if (frame->type == PJMEDIA_FRAME_TYPE_AUDIO && frame->size > 0) {
		samples = (const pj_int16_t *)frame->buf;
		count = (unsigned)(frame->size / sizeof(pj_int16_t));
	}
	if (g_cb.on_caller_audio)
		g_cb.on_caller_audio(g_cb.ud, samples, count);
	return PJ_SUCCESS;
}

/* Us -> conference: what the caller hears (the return feed, §3.3). Only
 * called while a call is connected to this port. */
static pj_status_t obs_get_frame(pjmedia_port *port, pjmedia_frame *frame)
{
	PJ_UNUSED_ARG(port);
	frame->size = SP_FRAME_SAMPLES * sizeof(pj_int16_t);
	if (g_cb.on_return_audio)
		g_cb.on_return_audio(g_cb.ud, (short *)frame->buf, SP_FRAME_SAMPLES);
	else
		pj_bzero(frame->buf, frame->size);
	frame->type = PJMEDIA_FRAME_TYPE_AUDIO;
	return PJ_SUCCESS;
}

static pj_status_t add_obs_port(void)
{
	pj_str_t name = pj_str("obs");

	/* The conference attaches a group lock to the port; after a restart
	 * (settings saved) that lock was freed with the old pjsua, so start
	 * from a clean struct every time. */
	pj_bzero(&g_obs_port, sizeof(g_obs_port));
	pjmedia_port_info_init(&g_obs_port.info, &name, PJMEDIA_SIG_CLASS_PORT_AUD('S', 'P'), SP_SAMPLE_RATE, 1, 16,
			       SP_FRAME_SAMPLES);
	g_obs_port.put_frame = &obs_put_frame;
	g_obs_port.get_frame = &obs_get_frame;
	return pjsua_conf_add_port(g_pool, &g_obs_port, &g_obs_slot);
}

/* --- calls ----------------------------------------------------------------- */

static void on_incoming_call(pjsua_acc_id acc_id, pjsua_call_id call_id, pjsip_rx_data *rdata)
{
	pjsua_call_info ci;
	char remote[256];

	PJ_UNUSED_ARG(acc_id);
	PJ_UNUSED_ARG(rdata);

	pjsua_call_answer(call_id, 180, NULL, NULL);
	if (pjsua_call_get_info(call_id, &ci) == PJ_SUCCESS)
		snprintf(remote, sizeof(remote), "%.*s", (int)ci.remote_info.slen, ci.remote_info.ptr);
	else
		remote[0] = '\0';

	if (g_cb.on_incoming)
		g_cb.on_incoming(g_cb.ud, call_id, remote);
}

static void on_call_state(pjsua_call_id call_id, pjsip_event *e)
{
	pjsua_call_info ci;

	PJ_UNUSED_ARG(e);
	if (pjsua_call_get_info(call_id, &ci) != PJ_SUCCESS || !g_cb.on_call_state)
		return;
	if (ci.state == PJSIP_INV_STATE_CONFIRMED)
		g_cb.on_call_state(g_cb.ud, call_id, SP_CALL_CONFIRMED);
	else if (ci.state == PJSIP_INV_STATE_DISCONNECTED)
		g_cb.on_call_state(g_cb.ud, call_id, SP_CALL_DISCONNECTED);
}

static void on_call_media_state(pjsua_call_id call_id)
{
	pjsua_call_info ci;

	if (pjsua_call_get_info(call_id, &ci) != PJ_SUCCESS)
		return;
	if (ci.media_status == PJSUA_CALL_MEDIA_ACTIVE && g_obs_slot != PJSUA_INVALID_ID) {
		/* Caller -> OBS, and the return feed OBS -> caller (silence when
		 * empty, which keeps RTP flowing). */
		pjsua_conf_connect(ci.conf_slot, g_obs_slot);
		pjsua_conf_connect(g_obs_slot, ci.conf_slot);
	}
}

void sp_answer(int call_id)
{
	ensure_thread();
	pjsua_call_answer(call_id, 200, NULL, NULL);
}

void sp_hangup(int call_id)
{
	ensure_thread();
	pjsua_call_hangup(call_id, 0, NULL, NULL);
}

void sp_reject_busy(int call_id)
{
	ensure_thread();
	pjsua_call_hangup(call_id, 486, NULL, NULL);
}

void sp_register(void)
{
	ensure_thread();
	if (g_acc != PJSUA_INVALID_ID)
		pjsua_acc_set_registration(g_acc, PJ_TRUE);
}

void sp_unregister(void)
{
	ensure_thread();
	if (g_acc != PJSUA_INVALID_ID) {
		g_unreg_pending = 1;
		if (pjsua_acc_set_registration(g_acc, PJ_FALSE) != PJ_SUCCESS)
			g_unreg_pending = 0;
	}
}

/* --- lifecycle --------------------------------------------------------------- */

static void set_codec_priorities(void)
{
	pj_str_t id;

	id = pj_str("*");
	pjsua_codec_set_priority(&id, 0);
	id = pj_str("opus/48000");
	pjsua_codec_set_priority(&id, 255);
	id = pj_str("G722/16000");
	pjsua_codec_set_priority(&id, 254);
	id = pj_str("PCMU/8000");
	pjsua_codec_set_priority(&id, 253);
	id = pj_str("PCMA/8000"); /* A-law: common with carriers outside North America */
	pjsua_codec_set_priority(&id, 252);
	id = pj_str("telephone-event");
	pjsua_codec_set_priority(&id, 128);
}

int sp_init(const sp_config *cfg, const sp_callbacks *cb, char *err, int err_len)
{
	pjsua_config ua;
	pjsua_logging_config log;
	pjsua_media_config media;
	pjsua_transport_config tcfg;
	pjsua_transport_id tp;
	pjsip_transport_type_e tp_type;
	pjsua_acc_config acc;
	char id[300];
	pj_status_t status;

	g_cb = *cb;
	g_proxy[0] = g_stun[0] = '\0';

	/* Route pjlib's start-up messages through the callback too. */
	pj_log_set_log_func(&on_log);
	pj_log_set_level(cfg->log_level);
	pj_log_set_decor(PJ_LOG_HAS_SENDER);

	status = pjsua_create();
	if (status != PJ_SUCCESS) {
		set_err(err, err_len, "pjsua_create", status);
		return -1;
	}

	pjsua_config_default(&ua);
	ua.max_calls = 4;
	ua.user_agent = pj_str("obs-softphone/0.1");
	ua.cb.on_reg_state2 = &on_reg_state2;
	ua.cb.on_incoming_call = &on_incoming_call;
	ua.cb.on_call_state = &on_call_state;
	ua.cb.on_call_media_state = &on_call_media_state;
	ua.cb.on_transport_state = &on_transport_state;
	if (cfg->stun_server && cfg->stun_server[0]) {
		snprintf(g_stun, sizeof(g_stun), "%s", cfg->stun_server);
		ua.stun_srv_cnt = 1;
		ua.stun_srv[0] = pj_str(g_stun);
		ua.stun_ignore_failure = PJ_TRUE;
	}

	pjsua_logging_config_default(&log);
	log.level = (unsigned)cfg->log_level;
	log.console_level = (unsigned)cfg->log_level;
	log.msg_logging = cfg->log_level >= 5 ? PJ_TRUE : PJ_FALSE;
	log.decor = PJ_LOG_HAS_SENDER;
	log.cb = &on_log;

	/* §3.1/§3.3: 48 kHz mono bridge, no echo canceller, no VAD. */
	pjsua_media_config_default(&media);
	media.clock_rate = 48000;
	media.snd_clock_rate = 48000;
	media.channel_count = 1;
	media.audio_frame_ptime = 20;
	media.ec_tail_len = 0;
	media.no_vad = PJ_TRUE;
	/* Keep the null device's clock running between calls, so OBS gets
	 * steady silence instead of no data (§3.2). */
	media.snd_auto_close_time = -1;

	status = pjsua_init(&ua, &log, &media);
	if (status != PJ_SUCCESS) {
		set_err(err, err_len, "pjsua_init", status);
		goto fail;
	}

	g_pool = pjsua_pool_create("sp", 1024, 1024);
	g_binding_pool = pjsua_pool_create("sp-binding", 512, 512);
	pj_mutex_create_simple(g_pool, "sp-binding", &g_binding_lock);

	/* Static module: clear what a previous pjsua left in it (restart). */
	g_options_sniffer.prev = g_options_sniffer.next = NULL;
	g_options_sniffer.id = -1;
	status = pjsip_endpt_register_module(pjsua_get_pjsip_endpt(), &g_options_sniffer);
	if (status != PJ_SUCCESS) {
		set_err(err, err_len, "register OPTIONS module", status);
		goto fail;
	}

	pjsua_transport_config_default(&tcfg);
	tcfg.port = 0;
	switch (cfg->transport) {
	case SP_TRANSPORT_UDP:
		tp_type = PJSIP_TRANSPORT_UDP;
		g_transport_param = "udp";
		break;
	case SP_TRANSPORT_TCP:
		tp_type = PJSIP_TRANSPORT_TCP;
		g_transport_param = "tcp";
		break;
	default:
		tp_type = PJSIP_TRANSPORT_TLS;
		g_transport_param = "tls";
		tcfg.tls_setting.proto = PJ_SSL_SOCK_PROTO_TLS1_2 | PJ_SSL_SOCK_PROTO_TLS1_3;
		tcfg.tls_setting.verify_server = cfg->verify_tls ? PJ_TRUE : PJ_FALSE;
		if (cfg->ca_file && cfg->ca_file[0])
			tcfg.tls_setting.ca_list_file = pj_str((char *)cfg->ca_file);
		break;
	}
	status = pjsua_transport_create(tp_type, &tcfg, &tp);
	if (status != PJ_SUCCESS) {
		set_err(err, err_len, "SIP transport", status);
		goto fail;
	}

	status = pjsua_start();
	if (status != PJ_SUCCESS) {
		set_err(err, err_len, "pjsua_start", status);
		goto fail;
	}
	pjsua_set_null_snd_dev();
	set_codec_priorities();

	status = add_obs_port();
	if (status != PJ_SUCCESS) {
		set_err(err, err_len, "add OBS media port", status);
		goto fail;
	}

	snprintf(g_username, sizeof(g_username), "%s", cfg->auth_username);
	snprintf(g_password, sizeof(g_password), "%s", cfg->password);
	snprintf(g_aor, sizeof(g_aor), "sip:%s@%s", cfg->username, cfg->domain);
	snprintf(g_srv_uri, sizeof(g_srv_uri), "sip:%s:%d;transport=%s", cfg->server, cfg->port, g_transport_param);
	snprintf(id, sizeof(id), "%s", g_aor);

	pjsua_acc_config_default(&acc);
	acc.id = pj_str(id);
	acc.reg_uri = pj_str(g_srv_uri);
	acc.register_on_acc_add = PJ_FALSE;
	acc.reg_timeout = 300;
	acc.reg_retry_interval = 0;          /* the core owns the backoff (§4.3) */
	/* Put our public address (learnt from the REGISTER response's
	 * received/rport) in Contact. Asterisk's rewrite_contact only fixes the
	 * registered contact; without this, the Contact of our 200 OK is a LAN
	 * address and Asterisk's ACK never arrives (call drops after 32 s). */
	acc.allow_contact_rewrite = PJ_TRUE;
	acc.contact_rewrite_method = PJSUA_CONTACT_REWRITE_NO_UNREG | PJSUA_CONTACT_REWRITE_ALWAYS_UPDATE;
	acc.use_rfc5626 = PJ_FALSE;
	acc.transport_id = tp;
	acc.cred_count = 1;
	acc.cred_info[0].realm = pj_str("*");
	acc.cred_info[0].scheme = pj_str("digest");
	acc.cred_info[0].username = pj_str(g_username);
	acc.cred_info[0].data_type = PJSIP_CRED_DATA_PLAIN_PASSWD;
	acc.cred_info[0].data = pj_str(g_password);
	if (cfg->outbound_proxy && cfg->outbound_proxy[0]) {
		snprintf(g_proxy, sizeof(g_proxy), "sip:%s;transport=%s;lr", cfg->outbound_proxy, g_transport_param);
		acc.proxy_cnt = 1;
		acc.proxy[0] = pj_str(g_proxy);
	}
	/* Encrypted media. Over TLS the keys travel encrypted; over UDP/TCP some
	 * providers still use SDES, so allow it there too. */
	acc.use_srtp = cfg->srtp == SP_SRTP_REQUIRED   ? PJMEDIA_SRTP_MANDATORY
		       : cfg->srtp == SP_SRTP_OPTIONAL ? PJMEDIA_SRTP_OPTIONAL
						       : PJMEDIA_SRTP_DISABLED;
	acc.srtp_secure_signaling = tp_type == PJSIP_TRANSPORT_TLS ? 1 : 0;

	status = pjsua_acc_add(&acc, PJ_TRUE, &g_acc);
	if (status != PJ_SUCCESS) {
		set_err(err, err_len, "add account", status);
		goto fail;
	}

	return 0;

fail:
	pjsua_destroy();
	g_acc = PJSUA_INVALID_ID;
	g_obs_slot = PJSUA_INVALID_ID;
	g_proxy[0] = g_stun[0] = '\0';
	return -1;
}

void sp_destroy(void)
{
	int i;

	ensure_thread();
	if (g_query) {
		pjsip_regc_destroy(g_query);
		g_query = NULL;
	}
	/* Let an unregistration already in flight finish (it may need an auth
	 * round trip); pjsua_destroy would otherwise start a second one and
	 * fail with "Object is busy". */
	for (i = 0; g_unreg_pending && i < 150; ++i)
		pj_thread_sleep(20);
	pjsua_destroy();
	g_acc = PJSUA_INVALID_ID;
	g_obs_slot = PJSUA_INVALID_ID;
	g_unreg_pending = 0;
	g_proxy[0] = g_stun[0] = '\0';
	g_binding = NULL;
	g_pool = NULL;
	g_binding_pool = NULL;
	g_binding_lock = NULL;
	memset(&g_cb, 0, sizeof(g_cb));
}

#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <wayland-client.h>
#include "config.h"
#include "ext-idle-notify-v1-client-protocol.h"
#include "wlr-foreign-toplevel-management-unstable-v1-client-protocol.h"

#if HAVE_LOGIND
#include <dbus/dbus.h>
#endif

typedef void (*dozed_idle_cb)(void *data, int idled);
typedef void (*dozed_logind_cb)(void *data, int event);

struct dozed_seat {
	struct wl_seat *proxy;
	char *name;
	uint32_t global_name;
};

struct dozed_wl {
	struct wl_display *display;
	struct wl_registry *registry;
	struct ext_idle_notifier_v1 *idle_notifier;
	struct zwlr_foreign_toplevel_manager_v1 *toplevel_manager;
	struct wl_seat *seat;
	struct dozed_seat *seats;
	size_t seat_count;
	size_t seat_cap;
	struct dozed_toplevel *toplevels;
	size_t toplevel_count;
	size_t toplevel_cap;
	int fullscreen_count;
	uint32_t idle_notifier_name;
	uint32_t toplevel_manager_name;
	int dead;
};

struct dozed_notification {
	struct ext_idle_notification_v1 *proxy;
	dozed_idle_cb callback;
	void *data;
};

struct dozed_toplevel {
	struct zwlr_foreign_toplevel_handle_v1 *proxy;
	struct dozed_wl *ctx;
	int fullscreen;
	int pending_fullscreen;
};

static void seat_handle_capabilities(void *data, struct wl_seat *seat,
		uint32_t capabilities) {
}

static void seat_handle_name(void *data, struct wl_seat *seat, const char *name) {
	struct dozed_seat *self = data;
	free(self->name);
	self->name = strdup(name);
}

static const struct wl_seat_listener seat_listener = {
	.capabilities = seat_handle_capabilities,
	.name = seat_handle_name,
};

static void handle_toplevel_title(void *data,
		struct zwlr_foreign_toplevel_handle_v1 *toplevel, const char *title) {
}

static void handle_toplevel_app_id(void *data,
		struct zwlr_foreign_toplevel_handle_v1 *toplevel, const char *app_id) {
}

static void handle_toplevel_output_enter(void *data,
		struct zwlr_foreign_toplevel_handle_v1 *toplevel,
		struct wl_output *output) {
}

static void handle_toplevel_output_leave(void *data,
		struct zwlr_foreign_toplevel_handle_v1 *toplevel,
		struct wl_output *output) {
}

static void handle_toplevel_state(void *data,
		struct zwlr_foreign_toplevel_handle_v1 *toplevel,
		struct wl_array *state) {
	struct dozed_toplevel *self = data;
	self->pending_fullscreen = 0;

	uint32_t *entry;
	wl_array_for_each(entry, state) {
		if (*entry == ZWLR_FOREIGN_TOPLEVEL_HANDLE_V1_STATE_FULLSCREEN) {
			self->pending_fullscreen = 1;
			break;
		}
	}
}

static void handle_toplevel_done(void *data,
		struct zwlr_foreign_toplevel_handle_v1 *toplevel) {
	struct dozed_toplevel *self = data;
	if (self->fullscreen == self->pending_fullscreen) {
		return;
	}
	self->fullscreen = self->pending_fullscreen;
	if (self->fullscreen) {
		self->ctx->fullscreen_count++;
	} else if (self->ctx->fullscreen_count > 0) {
		self->ctx->fullscreen_count--;
	}
}

static void destroy_toplevel(struct dozed_toplevel *self) {
	if (!self || !self->proxy) {
		return;
	}
	if (self->fullscreen && self->ctx->fullscreen_count > 0) {
		self->ctx->fullscreen_count--;
	}
	zwlr_foreign_toplevel_handle_v1_destroy(self->proxy);
	self->proxy = NULL;
	self->fullscreen = 0;
	self->pending_fullscreen = 0;
}

static void handle_toplevel_closed(void *data,
		struct zwlr_foreign_toplevel_handle_v1 *toplevel) {
	destroy_toplevel(data);
}

static void handle_toplevel_parent(void *data,
		struct zwlr_foreign_toplevel_handle_v1 *toplevel,
		struct zwlr_foreign_toplevel_handle_v1 *parent) {
}

static const struct zwlr_foreign_toplevel_handle_v1_listener toplevel_listener = {
	.title = handle_toplevel_title,
	.app_id = handle_toplevel_app_id,
	.output_enter = handle_toplevel_output_enter,
	.output_leave = handle_toplevel_output_leave,
	.state = handle_toplevel_state,
	.done = handle_toplevel_done,
	.closed = handle_toplevel_closed,
	.parent = handle_toplevel_parent,
};

static struct dozed_toplevel *push_toplevel(struct dozed_wl *ctx,
		struct zwlr_foreign_toplevel_handle_v1 *proxy) {
	if (ctx->toplevel_count == ctx->toplevel_cap) {
		size_t next = ctx->toplevel_cap ? ctx->toplevel_cap * 2 : 8;
		struct dozed_toplevel *toplevels =
			realloc(ctx->toplevels, next * sizeof(*ctx->toplevels));
		if (!toplevels) {
			zwlr_foreign_toplevel_handle_v1_destroy(proxy);
			return NULL;
		}
		ctx->toplevels = toplevels;
		ctx->toplevel_cap = next;
	}

	struct dozed_toplevel *toplevel =
		&ctx->toplevels[ctx->toplevel_count++];
	memset(toplevel, 0, sizeof(*toplevel));
	toplevel->ctx = ctx;
	toplevel->proxy = proxy;
	zwlr_foreign_toplevel_handle_v1_add_listener(proxy,
		&toplevel_listener, toplevel);
	return toplevel;
}

static void manager_handle_toplevel(void *data,
		struct zwlr_foreign_toplevel_manager_v1 *manager,
		struct zwlr_foreign_toplevel_handle_v1 *toplevel) {
	push_toplevel(data, toplevel);
}

static void manager_handle_finished(void *data,
		struct zwlr_foreign_toplevel_manager_v1 *manager) {
}

static const struct zwlr_foreign_toplevel_manager_v1_listener manager_listener = {
	.toplevel = manager_handle_toplevel,
	.finished = manager_handle_finished,
};

static void push_seat(struct dozed_wl *ctx, struct wl_registry *registry,
		uint32_t name) {
	if (ctx->seat_count == ctx->seat_cap) {
		size_t next = ctx->seat_cap ? ctx->seat_cap * 2 : 4;
		struct dozed_seat *seats =
			realloc(ctx->seats, next * sizeof(*ctx->seats));
		if (!seats) {
			return;
		}
		ctx->seats = seats;
		ctx->seat_cap = next;
	}

	struct dozed_seat *seat = &ctx->seats[ctx->seat_count++];
	memset(seat, 0, sizeof(*seat));
	seat->global_name = name;
	seat->proxy = wl_registry_bind(registry, name, &wl_seat_interface, 2);
	if (!seat->proxy) {
		ctx->seat_count--;
		return;
	}
	wl_seat_add_listener(seat->proxy, &seat_listener, seat);
}

static void registry_handle_global(void *data, struct wl_registry *registry,
		uint32_t name, const char *interface, uint32_t version) {
	struct dozed_wl *ctx = data;
	if (strcmp(interface, ext_idle_notifier_v1_interface.name) == 0) {
		uint32_t bind_version = version > 2 ? 2 : version;
		ctx->idle_notifier = wl_registry_bind(
			registry, name, &ext_idle_notifier_v1_interface, bind_version);
		if (ctx->idle_notifier) {
			ctx->idle_notifier_name = name;
		}
	} else if (strcmp(interface,
			zwlr_foreign_toplevel_manager_v1_interface.name) == 0) {
		uint32_t bind_version = version > 3 ? 3 : version;
		ctx->toplevel_manager = wl_registry_bind(registry, name,
			&zwlr_foreign_toplevel_manager_v1_interface, bind_version);
		if (ctx->toplevel_manager) {
			ctx->toplevel_manager_name = name;
			zwlr_foreign_toplevel_manager_v1_add_listener(
				ctx->toplevel_manager, &manager_listener, ctx);
		}
	} else if (strcmp(interface, wl_seat_interface.name) == 0) {
		push_seat(ctx, registry, name);
	}
}

static void registry_handle_global_remove(void *data,
		struct wl_registry *registry, uint32_t name) {
	struct dozed_wl *ctx = data;
	if (name == ctx->idle_notifier_name) {
		ctx->dead = 1;
		if (ctx->idle_notifier) {
			ext_idle_notifier_v1_destroy(ctx->idle_notifier);
			ctx->idle_notifier = NULL;
		}
		ctx->idle_notifier_name = 0;
		return;
	}
	if (name == ctx->toplevel_manager_name) {
		for (size_t i = 0; i < ctx->toplevel_count; ++i) {
			destroy_toplevel(&ctx->toplevels[i]);
		}
		free(ctx->toplevels);
		ctx->toplevels = NULL;
		ctx->toplevel_count = 0;
		ctx->toplevel_cap = 0;
		if (ctx->toplevel_manager) {
			zwlr_foreign_toplevel_manager_v1_destroy(ctx->toplevel_manager);
			ctx->toplevel_manager = NULL;
		}
		ctx->toplevel_manager_name = 0;
		ctx->fullscreen_count = 0;
		return;
	}
	for (size_t i = 0; i < ctx->seat_count; ++i) {
		if (ctx->seats[i].global_name == name) {
			free(ctx->seats[i].name);
			if (ctx->seats[i].proxy) {
				if (ctx->seat == ctx->seats[i].proxy) {
					ctx->seat = NULL;
					ctx->dead = 1;
				}
				wl_seat_destroy(ctx->seats[i].proxy);
			}
			memmove(&ctx->seats[i], &ctx->seats[i + 1],
				(ctx->seat_count - i - 1) * sizeof(*ctx->seats));
			ctx->seat_count--;
			return;
		}
	}
}

static const struct wl_registry_listener registry_listener = {
	.global = registry_handle_global,
	.global_remove = registry_handle_global_remove,
};

struct dozed_wl *dozed_wayland_connect(const char *seat_name) {
	struct dozed_wl *ctx = calloc(1, sizeof(*ctx));
	if (!ctx) {
		return NULL;
	}

	ctx->display = wl_display_connect(NULL);
	if (!ctx->display) {
		free(ctx);
		return NULL;
	}

	ctx->registry = wl_display_get_registry(ctx->display);
	wl_registry_add_listener(ctx->registry, &registry_listener, ctx);
	wl_display_roundtrip(ctx->display);
	wl_display_roundtrip(ctx->display);

	for (size_t i = 0; i < ctx->seat_count; ++i) {
		if (!seat_name || (ctx->seats[i].name &&
				strcmp(ctx->seats[i].name, seat_name) == 0)) {
			ctx->seat = ctx->seats[i].proxy;
		}
	}

	return ctx;
}

void dozed_wayland_destroy(struct dozed_wl *ctx) {
	if (!ctx) {
		return;
	}
	if (ctx->idle_notifier) {
		ext_idle_notifier_v1_destroy(ctx->idle_notifier);
	}
	for (size_t i = 0; i < ctx->toplevel_count; ++i) {
		destroy_toplevel(&ctx->toplevels[i]);
	}
	free(ctx->toplevels);
	if (ctx->toplevel_manager) {
		zwlr_foreign_toplevel_manager_v1_destroy(ctx->toplevel_manager);
	}
	for (size_t i = 0; i < ctx->seat_count; ++i) {
		free(ctx->seats[i].name);
		if (ctx->seats[i].proxy) {
			wl_seat_destroy(ctx->seats[i].proxy);
		}
	}
	free(ctx->seats);
	if (ctx->registry) {
		wl_registry_destroy(ctx->registry);
	}
	if (ctx->display) {
		wl_display_disconnect(ctx->display);
	}
	free(ctx);
}

int dozed_wayland_fd(struct dozed_wl *ctx) {
	if (!ctx || ctx->dead) {
		return -1;
	}
	return wl_display_get_fd(ctx->display);
}

int dozed_wayland_has_idle(struct dozed_wl *ctx) {
	return ctx && ctx->idle_notifier != NULL && !ctx->dead;
}

int dozed_wayland_has_seat(struct dozed_wl *ctx) {
	return ctx && ctx->seat != NULL && !ctx->dead;
}

int dozed_wayland_has_toplevel_manager(struct dozed_wl *ctx) {
	return ctx && ctx->toplevel_manager != NULL;
}

int dozed_wayland_has_fullscreen(struct dozed_wl *ctx) {
	return ctx && ctx->fullscreen_count > 0;
}

int dozed_wayland_flush(struct dozed_wl *ctx) {
	if (!ctx || ctx->dead) {
		return -1;
	}
	return wl_display_flush(ctx->display);
}

int dozed_wayland_dispatch(struct dozed_wl *ctx) {
	if (!ctx || ctx->dead) {
		return -1;
	}
	int ret = wl_display_dispatch(ctx->display);
	if (ret < 0) {
		return -errno;
	}
	return ret;
}

int dozed_wayland_roundtrip(struct dozed_wl *ctx) {
	if (!ctx || ctx->dead) {
		return -1;
	}
	return wl_display_roundtrip(ctx->display);
}

static void notification_idled(void *data,
		struct ext_idle_notification_v1 *notification) {
	struct dozed_notification *notif = data;
	notif->callback(notif->data, 1);
}

static void notification_resumed(void *data,
		struct ext_idle_notification_v1 *notification) {
	struct dozed_notification *notif = data;
	notif->callback(notif->data, 0);
}

static const struct ext_idle_notification_v1_listener notification_listener = {
	.idled = notification_idled,
	.resumed = notification_resumed,
};

struct dozed_notification *dozed_notification_create(
		struct dozed_wl *ctx, int timeout_ms, int obey_inhibitors,
		dozed_idle_cb callback, void *data) {
	if (!ctx || !ctx->idle_notifier || !ctx->seat || timeout_ms < 0 || ctx->dead) {
		return NULL;
	}

	struct dozed_notification *notif = calloc(1, sizeof(*notif));
	if (!notif) {
		return NULL;
	}
	notif->callback = callback;
	notif->data = data;

	uint32_t version = ext_idle_notifier_v1_get_version(ctx->idle_notifier);
	if (obey_inhibitors ||
			version < EXT_IDLE_NOTIFIER_V1_GET_INPUT_IDLE_NOTIFICATION_SINCE_VERSION) {
		notif->proxy = ext_idle_notifier_v1_get_idle_notification(
			ctx->idle_notifier, (uint32_t)timeout_ms, ctx->seat);
	} else {
		notif->proxy = ext_idle_notifier_v1_get_input_idle_notification(
			ctx->idle_notifier, (uint32_t)timeout_ms, ctx->seat);
	}
	if (!notif->proxy) {
		free(notif);
		return NULL;
	}
	ext_idle_notification_v1_add_listener(
		notif->proxy, &notification_listener, notif);
	return notif;
}

void dozed_notification_destroy(struct dozed_notification *notif) {
	if (!notif) {
		return;
	}
	if (notif->proxy) {
		ext_idle_notification_v1_destroy(notif->proxy);
	}
	free(notif);
}

#if HAVE_LOGIND
#define DBUS_LOGIND_SERVICE "org.freedesktop.login1"
#define DBUS_LOGIND_PATH "/org/freedesktop/login1"
#define DBUS_LOGIND_MANAGER_INTERFACE "org.freedesktop.login1.Manager"
#define DBUS_LOGIND_SESSION_INTERFACE "org.freedesktop.login1.Session"

struct dozed_logind {
	DBusConnection *bus;
	char *session_path;
	int sleep_lock_fd;
	dozed_logind_cb callback;
	void *data;
};

void dozed_logind_destroy(struct dozed_logind *ctx);

static int acquire_inhibitor_lock(struct dozed_logind *ctx,
		const char *type, const char *mode) {
	DBusError error;
	DBusMessage *msg = NULL;
	DBusMessage *reply = NULL;
	const char *who = "dozed";
	const char *why = "dozed is running idle hooks";
	int fd = -1;

	dbus_error_init(&error);
	msg = dbus_message_new_method_call(DBUS_LOGIND_SERVICE,
		DBUS_LOGIND_PATH, DBUS_LOGIND_MANAGER_INTERFACE, "Inhibit");
	if (!msg) {
		goto cleanup;
	}
	dbus_message_append_args(msg,
		DBUS_TYPE_STRING, &type,
		DBUS_TYPE_STRING, &who,
		DBUS_TYPE_STRING, &why,
		DBUS_TYPE_STRING, &mode,
		DBUS_TYPE_INVALID);
	reply = dbus_connection_send_with_reply_and_block(ctx->bus, msg, -1, &error);
	if (!reply) {
		goto cleanup;
	}
	dbus_message_get_args(reply, &error,
		DBUS_TYPE_UNIX_FD, &fd,
		DBUS_TYPE_INVALID);
	if (fd >= 0) {
		fd = fcntl(fd, F_DUPFD_CLOEXEC, 3);
	}

cleanup:
	dbus_error_free(&error);
	if (reply) {
		dbus_message_unref(reply);
	}
	if (msg) {
		dbus_message_unref(msg);
	}
	return fd;
}

static void release_sleep_lock(struct dozed_logind *ctx) {
	if (ctx->sleep_lock_fd >= 0) {
		close(ctx->sleep_lock_fd);
		ctx->sleep_lock_fd = -1;
	}
}

static int set_session(struct dozed_logind *ctx) {
	DBusError error;
	DBusMessage *msg = NULL;
	DBusMessage *reply = NULL;
	const char *auto_session = "auto";
	const char *path = NULL;
	uint32_t pid = getpid();
	int ret = -1;

	dbus_error_init(&error);
	msg = dbus_message_new_method_call(DBUS_LOGIND_SERVICE,
		DBUS_LOGIND_PATH, DBUS_LOGIND_MANAGER_INTERFACE, "GetSession");
	if (!msg) {
		goto cleanup;
	}
	dbus_message_append_args(msg,
		DBUS_TYPE_STRING, &auto_session,
		DBUS_TYPE_INVALID);
	reply = dbus_connection_send_with_reply_and_block(ctx->bus, msg, -1, &error);
	dbus_message_unref(msg);
	msg = NULL;

	if (!reply) {
		dbus_error_free(&error);
		dbus_error_init(&error);
		msg = dbus_message_new_method_call(DBUS_LOGIND_SERVICE,
			DBUS_LOGIND_PATH, DBUS_LOGIND_MANAGER_INTERFACE,
			"GetSessionByPID");
		if (!msg) {
			goto cleanup;
		}
		dbus_message_append_args(msg,
			DBUS_TYPE_UINT32, &pid,
			DBUS_TYPE_INVALID);
		reply = dbus_connection_send_with_reply_and_block(
			ctx->bus, msg, -1, &error);
		if (!reply) {
			goto cleanup;
		}
	}

	if (dbus_message_get_args(reply, &error,
			DBUS_TYPE_OBJECT_PATH, &path,
			DBUS_TYPE_INVALID)) {
		ctx->session_path = strdup(path);
		if (!ctx->session_path) {
			ret = -1;
		} else {
			ret = 0;
		}
	}

cleanup:
	dbus_error_free(&error);
	if (reply) {
		dbus_message_unref(reply);
	}
	if (msg) {
		dbus_message_unref(msg);
	}
	return ret;
}

static DBusHandlerResult logind_filter(DBusConnection *connection,
		DBusMessage *msg, void *userdata) {
	struct dozed_logind *ctx = userdata;

	if (dbus_message_is_signal(msg, DBUS_LOGIND_MANAGER_INTERFACE,
			"PrepareForSleep")) {
		DBusError error;
		dbus_bool_t going_down = TRUE;
		dbus_error_init(&error);
		if (!dbus_message_get_args(msg, &error,
				DBUS_TYPE_BOOLEAN, &going_down,
				DBUS_TYPE_INVALID)) {
			dbus_error_free(&error);
			return DBUS_HANDLER_RESULT_HANDLED;
		}
		if (going_down) {
			if (ctx->callback) {
				ctx->callback(ctx->data, 1);
			}
			release_sleep_lock(ctx);
		} else {
			ctx->sleep_lock_fd = acquire_inhibitor_lock(ctx, "sleep", "delay");
			if (ctx->callback) {
				ctx->callback(ctx->data, 2);
			}
		}
		return DBUS_HANDLER_RESULT_HANDLED;
	}

	if (dbus_message_is_signal(msg, DBUS_LOGIND_SESSION_INTERFACE, "Lock")) {
		if (ctx->callback) {
			ctx->callback(ctx->data, 3);
		}
		return DBUS_HANDLER_RESULT_HANDLED;
	}

	if (dbus_message_is_signal(msg, DBUS_LOGIND_SESSION_INTERFACE, "Unlock")) {
		if (ctx->callback) {
			ctx->callback(ctx->data, 4);
		}
		return DBUS_HANDLER_RESULT_HANDLED;
	}

	return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
}

static void add_match(struct dozed_logind *ctx, const char *rule) {
	DBusError error;
	dbus_error_init(&error);
	dbus_bus_add_match(ctx->bus, rule, &error);
	dbus_connection_flush(ctx->bus);
	dbus_error_free(&error);
}

static char *session_match_rule(const char *session_path, const char *member) {
	const char *prefix =
		"type='signal',sender='" DBUS_LOGIND_SERVICE
		"',interface='" DBUS_LOGIND_SESSION_INTERFACE "',path='";
	const char *middle = "',member='";
	const char *suffix = "'";
	size_t len = strlen(prefix) + strlen(session_path) + strlen(middle) +
		strlen(member) + strlen(suffix) + 1;
	char *rule = malloc(len);
	if (rule) {
		snprintf(rule, len, "%s%s%s%s%s", prefix, session_path, middle,
			member, suffix);
	}
	return rule;
}

struct dozed_logind *dozed_logind_connect(int want_sleep,
		int want_lock, int want_unlock, dozed_logind_cb callback,
		void *data) {
	DBusError error;
	struct dozed_logind *ctx = calloc(1, sizeof(*ctx));
	if (!ctx) {
		return NULL;
	}
	ctx->sleep_lock_fd = -1;
	ctx->callback = callback;
	ctx->data = data;

	dbus_error_init(&error);
	ctx->bus = dbus_bus_get(DBUS_BUS_SYSTEM, &error);
	if (!ctx->bus || set_session(ctx) < 0) {
		dbus_error_free(&error);
		dozed_logind_destroy(ctx);
		return NULL;
	}
	dbus_error_free(&error);
	dbus_connection_set_exit_on_disconnect(ctx->bus, FALSE);
	dbus_connection_add_filter(ctx->bus, logind_filter, ctx, NULL);

	if (want_sleep) {
		add_match(ctx,
			"type='signal',sender='" DBUS_LOGIND_SERVICE
			"',interface='" DBUS_LOGIND_MANAGER_INTERFACE
			"',path='" DBUS_LOGIND_PATH "',member='PrepareForSleep'");
		ctx->sleep_lock_fd = acquire_inhibitor_lock(ctx, "sleep", "delay");
	}
	if (want_lock) {
		char *rule = session_match_rule(ctx->session_path, "Lock");
		if (rule) {
			add_match(ctx, rule);
			free(rule);
		}
	}
	if (want_unlock) {
		char *rule = session_match_rule(ctx->session_path, "Unlock");
		if (rule) {
			add_match(ctx, rule);
			free(rule);
		}
	}
	return ctx;
}

void dozed_logind_destroy(struct dozed_logind *ctx) {
	if (!ctx) {
		return;
	}
	release_sleep_lock(ctx);
	free(ctx->session_path);
	if (ctx->bus) {
		dbus_connection_remove_filter(ctx->bus, logind_filter, ctx);
		dbus_connection_unref(ctx->bus);
	}
	free(ctx);
}

int dozed_logind_fd(struct dozed_logind *ctx) {
	int fd = -1;
	if (!ctx || !ctx->bus) {
		return -1;
	}
	if (!dbus_connection_get_unix_fd(ctx->bus, &fd)) {
		return -1;
	}
	return fd;
}

int dozed_logind_process(struct dozed_logind *ctx) {
	if (!ctx || !ctx->bus) {
		return -1;
	}
	if (!dbus_connection_read_write(ctx->bus, 0)) {
		return -1;
	}
	while (dbus_connection_dispatch(ctx->bus) == DBUS_DISPATCH_DATA_REMAINS) {
	}
	dbus_connection_flush(ctx->bus);
	return 0;
}
#else
struct dozed_logind { int unused; };

struct dozed_logind *dozed_logind_connect(int want_sleep,
		int want_lock, int want_unlock, dozed_logind_cb callback,
		void *data) {
	return NULL;
}

void dozed_logind_destroy(struct dozed_logind *ctx) {
}

int dozed_logind_fd(struct dozed_logind *ctx) {
	return -1;
}

int dozed_logind_process(struct dozed_logind *ctx) {
	return 0;
}
#endif

// SPDX-License-Identifier: MIT
// Small protocol exerciser; commands arrive on stdin, events on stdout.
#define _GNU_SOURCE
#include <assert.h>
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>
#include <wayland-client.h>
#include "xdg-shell-client-protocol.h"
#include "pointer-constraints-client-protocol.h"
#include "relative-pointer-client-protocol.h"
#include "wlr-layer-shell-client-protocol.h"

static struct wl_display *display;
static struct wl_compositor *compositor;
static struct wl_shm *shm;
static struct wl_seat *seat;
static struct zwlr_layer_shell_v1 *layer_shell;
static int panel;
static struct wl_pointer *pointer;
static struct wl_surface *surface;
static struct xdg_wm_base *shell;
static struct xdg_toplevel *toplevel;
static struct zwp_pointer_constraints_v1 *constraints;
static struct zwp_relative_pointer_manager_v1 *relative_manager;
static struct zwp_confined_pointer_v1 *confined;
static struct zwp_locked_pointer_v1 *locked;
static int width = 600, height = 400;
static void buffer_release(void *data, struct wl_buffer *buffer) { (void)data; wl_buffer_destroy(buffer); }
static const struct wl_buffer_listener buffer_listener = {.release = buffer_release};
static void draw(void) {
    int size = width * height * 4;
    int fd = memfd_create("minde-pointer-test", MFD_CLOEXEC);
    assert(fd >= 0 && ftruncate(fd, size) == 0);
    uint32_t *pixels = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    assert(pixels != MAP_FAILED);
    for (int i = 0; i < width * height; ++i) pixels[i] = panel ? 0xffa02020 : 0xff204060;
    struct wl_shm_pool *pool = wl_shm_create_pool(shm, fd, size);
    struct wl_buffer *buffer = wl_shm_pool_create_buffer(pool, 0, width, height, width * 4, WL_SHM_FORMAT_XRGB8888);
    wl_buffer_add_listener(buffer, &buffer_listener, NULL);
    wl_shm_pool_destroy(pool); munmap(pixels, size); close(fd);
    wl_surface_attach(surface, buffer, 0, 0);
    wl_surface_damage(surface, 0, 0, width, height);
    wl_surface_commit(surface);
}
static void layer_configure(void *d, struct zwlr_layer_surface_v1 *s, uint32_t serial, uint32_t w, uint32_t h) {
    (void)d; width = (int)w; height = (int)h; zwlr_layer_surface_v1_ack_configure(s, serial); draw(); puts("panel-ready");
}
static void layer_closed(void *d, struct zwlr_layer_surface_v1 *s) { (void)d; (void)s; exit(0); }
static const struct zwlr_layer_surface_v1_listener layer_listener = {.configure=layer_configure, .closed=layer_closed};
static void ping(void *d, struct xdg_wm_base *s, uint32_t serial) { (void)d; xdg_wm_base_pong(s, serial); }
static const struct xdg_wm_base_listener shell_listener = {.ping = ping};
static void configure(void *d, struct xdg_surface *s, uint32_t serial) {
    (void)d; xdg_surface_ack_configure(s, serial); draw();
    printf("size %d %d\n", width, height);
}
static const struct xdg_surface_listener surface_listener = {.configure = configure};
static void top_configure(void *d, struct xdg_toplevel *t, int32_t w, int32_t h, struct wl_array *states) {
    (void)d; (void)t; (void)states; if (w > 0) width = w; if (h > 0) height = h;
}
static void top_close(void *d, struct xdg_toplevel *t) { (void)d; (void)t; exit(0); }
static const struct xdg_toplevel_listener top_listener = {.configure = top_configure, .close = top_close};
static void enter(void *d, struct wl_pointer *p, uint32_t serial, struct wl_surface *s, wl_fixed_t x, wl_fixed_t y) {
    (void)d; (void)p; (void)serial; (void)s; printf("enter %.3f %.3f\n", wl_fixed_to_double(x), wl_fixed_to_double(y));
}
static void leave(void *d, struct wl_pointer *p, uint32_t serial, struct wl_surface *s) {
    (void)d; (void)p; (void)serial; (void)s; puts("leave");
}
static void motion(void *d, struct wl_pointer *p, uint32_t time, wl_fixed_t x, wl_fixed_t y) {
    (void)d; (void)p; (void)time; printf("motion %.3f %.3f\n", wl_fixed_to_double(x), wl_fixed_to_double(y));
}
static void button(void *d, struct wl_pointer *p, uint32_t serial, uint32_t time, uint32_t b, uint32_t state) {
    (void)d; (void)p; (void)serial; (void)time; (void)b; (void)state;
}
static void axis(void *d, struct wl_pointer *p, uint32_t time, uint32_t a, wl_fixed_t value) {
    (void)d; (void)p; (void)time; (void)a; (void)value;
}
static const struct wl_pointer_listener pointer_listener = {.enter=enter, .leave=leave, .motion=motion, .button=button, .axis=axis};
static void keyboard_keymap(void *d, struct wl_keyboard *k, uint32_t format, int32_t fd, uint32_t size) {
    (void)d; (void)k; (void)format; (void)size; close(fd);
}
static void keyboard_enter(void *d, struct wl_keyboard *k, uint32_t serial, struct wl_surface *s, struct wl_array *keys) {
    (void)d; (void)k; (void)serial; (void)s; (void)keys; puts("keyboard-enter");
}
static void keyboard_leave(void *d, struct wl_keyboard *k, uint32_t serial, struct wl_surface *s) {
    (void)d; (void)k; (void)serial; (void)s; puts("keyboard-leave");
}
static void keyboard_key(void *d, struct wl_keyboard *k, uint32_t serial, uint32_t time, uint32_t key, uint32_t state) {
    (void)d; (void)k; (void)serial; (void)time; printf("key %u %u\n", key, state);
}
static void keyboard_modifiers(void *d, struct wl_keyboard *k, uint32_t serial, uint32_t depressed, uint32_t latched, uint32_t locked_mods, uint32_t group) {
    (void)d; (void)k; (void)serial; (void)depressed; (void)latched; (void)locked_mods; (void)group;
}
static const struct wl_keyboard_listener keyboard_listener = {
    .keymap=keyboard_keymap, .enter=keyboard_enter, .leave=keyboard_leave,
    .key=keyboard_key, .modifiers=keyboard_modifiers
};
static void on_confined(void *d, struct zwp_confined_pointer_v1 *p) { (void)d; (void)p; puts("confined"); }
static void on_unconfined(void *d, struct zwp_confined_pointer_v1 *p) { (void)d; (void)p; puts("unconfined"); }
static const struct zwp_confined_pointer_v1_listener confine_listener = {.confined=on_confined, .unconfined=on_unconfined};
static void on_locked(void *d, struct zwp_locked_pointer_v1 *p) { (void)d; (void)p; puts("locked"); }
static void on_unlocked(void *d, struct zwp_locked_pointer_v1 *p) { (void)d; (void)p; puts("unlocked"); }
static const struct zwp_locked_pointer_v1_listener lock_listener = {.locked=on_locked, .unlocked=on_unlocked};
static void relative(void *d, struct zwp_relative_pointer_v1 *p, uint32_t hi, uint32_t lo, wl_fixed_t dx, wl_fixed_t dy, wl_fixed_t ux, wl_fixed_t uy) {
    (void)d; (void)p; (void)hi; (void)lo; (void)ux; (void)uy;
    printf("relative %.3f %.3f\n", wl_fixed_to_double(dx), wl_fixed_to_double(dy));
}
static const struct zwp_relative_pointer_v1_listener relative_listener = {.relative_motion=relative};
static void global(void *d, struct wl_registry *r, uint32_t name, const char *interface, uint32_t version) {
    (void)d; (void)version;
#define BIND(field, type, ver) if (!strcmp(interface, #type)) field = wl_registry_bind(r, name, &type##_interface, ver)
    BIND(compositor, wl_compositor, 1);
    BIND(layer_shell, zwlr_layer_shell_v1, 1);
    BIND(shm, wl_shm, 1);
    BIND(seat, wl_seat, 1);
    BIND(shell, xdg_wm_base, 1);
    BIND(constraints, zwp_pointer_constraints_v1, 1);
    BIND(relative_manager, zwp_relative_pointer_manager_v1, 1);
#undef BIND
}
static void global_remove(void *d, struct wl_registry *r, uint32_t name) { (void)d; (void)r; (void)name; }
static const struct wl_registry_listener registry_listener = {.global=global, .global_remove=global_remove};
static void command(const char *cmd) {
    if (!strcmp(cmd, "release")) {
        if (confined) { zwp_confined_pointer_v1_destroy(confined); confined = NULL; }
        if (locked) { zwp_locked_pointer_v1_destroy(locked); locked = NULL; }
        wl_surface_set_input_region(surface, NULL); wl_surface_commit(surface);
    } else if (!strncmp(cmd, "confine", 7)) {
        struct wl_region *region = NULL;
        if (strcmp(cmd, "confine-null")) {
            region = wl_compositor_create_region(compositor);
            wl_region_add(region, 0, 0, 200, 200);
            if (!strcmp(cmd, "confine-hole")) wl_region_subtract(region, 80, 0, 40, 200);
            if (!strcmp(cmd, "confine-input")) {
                wl_surface_set_input_region(surface, region); wl_surface_commit(surface);
                wl_region_destroy(region); region = NULL;
            }
        }
        confined = zwp_pointer_constraints_v1_confine_pointer(constraints, surface, pointer, region, ZWP_POINTER_CONSTRAINTS_V1_LIFETIME_PERSISTENT);
        zwp_confined_pointer_v1_add_listener(confined, &confine_listener, NULL);
        if (region) wl_region_destroy(region);
    } else if (!strcmp(cmd, "lock")) {
        locked = zwp_pointer_constraints_v1_lock_pointer(constraints, surface, pointer, NULL, ZWP_POINTER_CONSTRAINTS_V1_LIFETIME_PERSISTENT);
        zwp_locked_pointer_v1_add_listener(locked, &lock_listener, NULL);
    } else if (!strcmp(cmd, "hint")) {
        assert(locked); zwp_locked_pointer_v1_set_cursor_position_hint(locked, wl_fixed_from_int(50), wl_fixed_from_int(60)); wl_surface_commit(surface);
    } else if (!strcmp(cmd, "fullscreen")) xdg_toplevel_set_fullscreen(toplevel, NULL);
    else if (!strcmp(cmd, "unfullscreen")) xdg_toplevel_unset_fullscreen(toplevel);
    else if (!strcmp(cmd, "quit")) exit(0);
    else { fprintf(stderr, "unknown command: %s\n", cmd); exit(2); }
    assert(wl_display_roundtrip(display) >= 0);
    printf("done %s\n", cmd);
}
int main(int argc, char **argv) {
    setbuf(stdout, NULL);
    display = wl_display_connect(NULL); assert(display);
    struct wl_registry *registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, NULL);
    assert(wl_display_roundtrip(display) >= 0);
    assert(compositor && shm && seat && shell && constraints && relative_manager);
    xdg_wm_base_add_listener(shell, &shell_listener, NULL);
    struct wl_keyboard *keyboard = wl_seat_get_keyboard(seat);
    wl_keyboard_add_listener(keyboard, &keyboard_listener, NULL);
    pointer = wl_seat_get_pointer(seat); wl_pointer_add_listener(pointer, &pointer_listener, NULL);
    struct zwp_relative_pointer_v1 *rp = zwp_relative_pointer_manager_v1_get_relative_pointer(relative_manager, pointer);
    zwp_relative_pointer_v1_add_listener(rp, &relative_listener, NULL);
    surface = wl_compositor_create_surface(compositor);
    panel = argc > 1 && !strcmp(argv[1], "panel");
    if (panel) {
        assert(layer_shell);
        struct zwlr_layer_surface_v1 *layer = zwlr_layer_shell_v1_get_layer_surface(layer_shell, surface, NULL, ZWLR_LAYER_SHELL_V1_LAYER_TOP, "minde-test-panel");
        zwlr_layer_surface_v1_add_listener(layer, &layer_listener, NULL);
        zwlr_layer_surface_v1_set_size(layer, 0, 34);
        zwlr_layer_surface_v1_set_anchor(layer, ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP | ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT | ZWLR_LAYER_SURFACE_V1_ANCHOR_RIGHT);
        zwlr_layer_surface_v1_set_exclusive_zone(layer, 34);
    } else {
    struct xdg_surface *xdg = xdg_wm_base_get_xdg_surface(shell, surface);
    xdg_surface_add_listener(xdg, &surface_listener, NULL);
    toplevel = xdg_surface_get_toplevel(xdg);
    xdg_toplevel_add_listener(toplevel, &top_listener, NULL);
    xdg_toplevel_set_title(toplevel, "minde-pointer-test");
    xdg_toplevel_set_app_id(toplevel, "minde-pointer-test");
    }
    wl_surface_commit(surface);
    char line[128]; size_t used = 0;
    for (;;) {
        assert(wl_display_dispatch_pending(display) >= 0);
        assert(wl_display_flush(display) >= 0);
        struct pollfd fds[] = {{wl_display_get_fd(display), POLLIN, 0}, {STDIN_FILENO, POLLIN, 0}};
        assert(poll(fds, 2, -1) >= 0);
        if (fds[0].revents & POLLIN) assert(wl_display_dispatch(display) >= 0);
        if (fds[1].revents & POLLIN) {
            char c; if (read(STDIN_FILENO, &c, 1) != 1) break;
            if (c == '\n') { line[used] = 0; command(line); used = 0; }
            else { assert(used < sizeof(line) - 1); line[used++] = c; }
        }
        if (fds[1].revents & POLLHUP) break;
    }
    return 0;
}

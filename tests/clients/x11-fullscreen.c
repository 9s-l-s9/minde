// SPDX-License-Identifier: MIT
// Exercise startup and runtime EWMH fullscreen, plus client self-resize.
#include <X11/Xlib.h>
#include <X11/Xatom.h>
#include <assert.h>
#include <poll.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
int main(int argc, char **argv) {
    setbuf(stdout, NULL);
    Display *d = XOpenDisplay(NULL); assert(d);
    Window root = DefaultRootWindow(d);
    Window w = XCreateSimpleWindow(d, root, 0, 0, 640, 480, 0, 0, 0x204060);
    Atom state = XInternAtom(d, "_NET_WM_STATE", False);
    Atom fs = XInternAtom(d, "_NET_WM_STATE_FULLSCREEN", False);
    XStoreName(d, w, "minde-fullscreen-test");
    XSelectInput(d, w, StructureNotifyMask | PointerMotionMask);
    if (argc > 1 && !strcmp(argv[1], "fullscreen")) XChangeProperty(d, w, state, XA_ATOM, 32, PropModeReplace, (unsigned char *)&fs, 1);
    XMapWindow(d, w); XFlush(d);
    char cmd[100]; size_t used = 0;
    for (;;) {
        while (XPending(d)) {
            XEvent event; XNextEvent(d, &event);
            if (event.type == ConfigureNotify) printf("geometry %d %d %d %d\n", event.xconfigure.x, event.xconfigure.y, event.xconfigure.width, event.xconfigure.height);
        }
        struct pollfd fds[] = {{ConnectionNumber(d), POLLIN, 0}, {STDIN_FILENO, POLLIN, 0}};
        assert(poll(fds, 2, -1) >= 0);
        if (fds[1].revents & POLLIN) {
            char c; if (read(STDIN_FILENO, &c, 1) != 1) break;
            if (c == '\n') {
                cmd[used] = 0; used = 0;
                if (!strcmp(cmd, "grab")) {
                    int status = XGrabPointer(d, w, True, PointerMotionMask | ButtonPressMask | ButtonReleaseMask, GrabModeAsync, GrabModeAsync, w, None, CurrentTime);
                    printf("grab %d\n", status);
                } else if (!strcmp(cmd, "ungrab")) XUngrabPointer(d, CurrentTime);
                else if (!strcmp(cmd, "resize")) XResizeWindow(d, w, 320, 240);
                else if (!strcmp(cmd, "quit")) break;
                else {
                    XEvent event = {0}; event.xclient.type = ClientMessage;
                    event.xclient.window = w; event.xclient.message_type = state; event.xclient.format = 32;
                    event.xclient.data.l[0] = !strcmp(cmd, "fullscreen"); event.xclient.data.l[1] = fs; event.xclient.data.l[3] = 1;
                    XSendEvent(d, root, False, SubstructureRedirectMask | SubstructureNotifyMask, &event);
                }
                XSync(d, False); printf("done %s\n", cmd);
            } else { assert(used < sizeof(cmd)-1); cmd[used++] = c; }
        }
        if (fds[1].revents & POLLHUP) break;
    }
    XDestroyWindow(d, w); XCloseDisplay(d); return 0;
}

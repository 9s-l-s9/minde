/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Test-only libEGL.so.1 proxy. All functions delegate to the selected real
 * EGL except an armed failure targeting the nested compositor's PID.
 * The control file contains: render|swap temporary|fatal once|hold PID. */
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static _Thread_local EGLint pending_error;
static _Thread_local EGLSurface retired_surface;
static _Thread_local EGLDisplay retired_display;

__attribute__((constructor)) static void loaded(void)
{
    fprintf(stderr, "egl-fault: proxy loaded\n");
}

static void *real_symbol(const char *name)
{
    static void *library;
    if (!library) {
        const char *path = getenv("MINDE_TEST_REAL_EGL");
        if (!path || !(library = dlopen(path, RTLD_NOW | RTLD_LOCAL))) {
            fprintf(stderr, "egl-fault: cannot load real EGL: %s\n", dlerror());
            abort();
        }
    }
    void *symbol = dlsym(library, name);
    if (!symbol) {
        PFNEGLGETPROCADDRESSPROC get = dlsym(library, "eglGetProcAddress");
        symbol = (void *)get(name);
    }
    return symbol;
}

static int inject(const char *stage)
{
    const char *path = getenv("MINDE_TEST_EGL_FAULT");
    FILE *file = path ? fopen(path, "r") : NULL;
    if (!file)
        return 0;
    char selected[16], kind[16], repeat[16];
    long target;
    int fields = fscanf(file, "%15s %15s %15s %ld", selected, kind, repeat, &target);
    fclose(file);
    if (fields != 4 || target != (long)getpid() || strcmp(selected, stage))
        return 0;
    if (strcmp(repeat, "hold"))
        unlink(path);
    pending_error = !strcmp(kind, "fatal") ? EGL_CONTEXT_LOST :
        (!strcmp(stage, "swap") ? EGL_BAD_SURFACE : EGL_BAD_ACCESS);
    fprintf(stderr, "egl-fault: injected %s %s\n", stage, kind);
    return 1;
}

EGLint eglGetError(void)
{
    if (pending_error) {
        EGLint error = pending_error;
        pending_error = 0;
        return error;
    }
    PFNEGLGETERRORPROC get = real_symbol("eglGetError");
    return get();
}

EGLBoolean eglMakeCurrent(EGLDisplay display, EGLSurface draw,
                          EGLSurface read, EGLContext context)
{
    if (draw != EGL_NO_SURFACE && context != EGL_NO_CONTEXT && inject("render"))
        return EGL_FALSE;
    PFNEGLMAKECURRENTPROC make = real_symbol("eglMakeCurrent");
    return make(display, draw, read, context);
}

static int fail_swap(EGLDisplay display, EGLSurface surface)
{
    if (!inject("swap"))
        return 0;
    if (pending_error == EGL_BAD_SURFACE) {
        /* Really retire the native surface. Smithay will recreate it; merely
         * reporting BadSurface while it still exists can yield BadAlloc when
         * a driver refuses two EGL surfaces for the same native window. */
        PFNEGLMAKECURRENTPROC make = real_symbol("eglMakeCurrent");
        PFNEGLDESTROYSURFACEPROC destroy = real_symbol("eglDestroySurface");
        make(display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
        destroy(display, surface);
        retired_display = display;
        retired_surface = surface;
    }
    return 1;
}

EGLBoolean eglDestroySurface(EGLDisplay display, EGLSurface surface)
{
    if (display == retired_display && surface == retired_surface) {
        /* We already destroyed this handle. Mesa may have reused its address
         * for the replacement; Smithay's old-surface cleanup must not destroy
         * that new surface a second time. */
        retired_surface = EGL_NO_SURFACE;
        return EGL_TRUE;
    }
    PFNEGLDESTROYSURFACEPROC destroy = real_symbol("eglDestroySurface");
    return destroy(display, surface);
}

EGLBoolean eglSwapBuffers(EGLDisplay display, EGLSurface surface)
{
    if (fail_swap(display, surface))
        return EGL_FALSE;
    PFNEGLSWAPBUFFERSPROC swap = real_symbol("eglSwapBuffers");
    return swap(display, surface);
}

EGLBoolean eglSwapBuffersWithDamageEXT(EGLDisplay display, EGLSurface surface,
                                      const EGLint *rects, EGLint count)
{
    if (fail_swap(display, surface))
        return EGL_FALSE;
    PFNEGLSWAPBUFFERSWITHDAMAGEEXTPROC swap = real_symbol("eglSwapBuffersWithDamageEXT");
    return swap(display, surface, rects, count);
}

EGLBoolean eglSwapBuffersWithDamageKHR(EGLDisplay display, EGLSurface surface,
                                      const EGLint *rects, EGLint count)
{
    if (fail_swap(display, surface))
        return EGL_FALSE;
    PFNEGLSWAPBUFFERSWITHDAMAGEKHRPROC swap = real_symbol("eglSwapBuffersWithDamageKHR");
    return swap(display, surface, rects, count);
}

__eglMustCastToProperFunctionPointerType eglGetProcAddress(const char *name)
{
#define WRAP(symbol) if (!strcmp(name, #symbol)) return (__eglMustCastToProperFunctionPointerType)symbol
    /* Smithay reloads this entry point while resolving the rest of EGL. */
    WRAP(eglGetProcAddress);
    WRAP(eglGetError);
    WRAP(eglDestroySurface);
    WRAP(eglMakeCurrent);
    WRAP(eglSwapBuffers);
    WRAP(eglSwapBuffersWithDamageEXT);
    WRAP(eglSwapBuffersWithDamageKHR);
#undef WRAP
    return (__eglMustCastToProperFunctionPointerType)real_symbol(name);
}

;;; guix.scm -- build minde as a Guix package, including the SDDM
;;; session entry.
;;;
;;; The crate graph (including the pinned smithay git checkout) is vendored
;;; rather than imported crate-by-crate. One-time step after dependency
;;; changes, from the repo root:
;;;
;;;     guix shell -m manifest.scm -- cargo vendor vendor
;;;
;;; then:
;;;
;;;     guix build -f guix.scm
;;;
;;; The build is fully offline against vendor/.

(use-modules (guix packages)
             (guix gexp)
             (guix build-system gnu)
             (srfi srfi-1)
             (ice-9 popen)
             (ice-9 ftw)
             (ice-9 textual-ports)
             ((guix licenses) #:prefix license:)
             (gnu packages base)
             (gnu packages rust)
             (gnu packages pkg-config)
             (gnu packages guile)
             (gnu packages admin)
             (gnu packages freedesktop)
             (gnu packages xdisorg)
             (gnu packages xorg)
             (gnu packages gl)
             (gnu packages linux)
             (gnu packages bash))

(define %source-dir (dirname (current-filename)))
(define %project-version "1.0.0-rc1")
(define %source-archive (getenv "MINDE_SOURCE_ARCHIVE"))
(define %build-revision
  (or (getenv "MINDE_BUILD_REVISION") "local-checkout"))

;; A dirty checkout ships whatever is on disk -- including half-finished
;; edits -- straight into the system generation via local-file. Warn
;; loudly so a reconfigure never bakes in work-in-progress silently.
;; (The archive path is exempt: it builds from a fixed tarball.)
(unless %source-archive
  (let* ((port (open-input-pipe
                (string-append "git -C " %source-dir
                               " status --porcelain 2>/dev/null")))
         (dirt (get-string-all port)))
    (close-pipe port)
    (unless (or (eof-object? dirt) (string-null? (string-trim-both dirt)))
      (format (current-error-port) "~%minde guix.scm: WARNING: \
building from a DIRTY working tree:~%~a\
Uncommitted changes will be baked into this build.  Commit or stash \
first, or set MINDE_SOURCE_ARCHIVE for a known-good artifact.~%~%"
              dirt))))

(define (source-select? file stat)
  ;; Everything except top-level build/agent artifacts and VCS metadata. Only
  ;; these repo-root paths are excluded -- vendored crates legitimately contain
  ;; paths such as vendor/cc/src/target/. vendor/ itself IS included; it is the
  ;; offline crate mirror.
  (let ((rel (string-drop file (+ 1 (string-length %source-dir)))))
    (not (any (lambda (directory)
                (or (string=? rel directory)
                    (string-prefix? (string-append directory "/") rel)))
              '("target" ".git" ".local" ".cache" ".claude"
                ".agents" ".codex" "build")))))

(package
  (name "minde")
  (version (or (getenv "MINDE_VERSION") %project-version))
  (source
   (if %source-archive
       (local-file %source-archive
                   (string-append "minde-" version "-vendored.tar.gz"))
       (local-file %source-dir "minde-source"
                   #:recursive? #t
                   #:select? source-select?)))
  (build-system gnu-build-system)
  (arguments
   (list
    #:tests? #t
    #:phases
    #~(modify-phases %standard-phases
        (delete 'configure)
        ;; Shebang patching rewrites scripts inside vendor/, invalidating
        ;; cargo's .cargo-checksum.json for those crates.
        (delete 'patch-source-shebangs)
        (delete 'patch-generated-file-shebangs)
        (add-after 'unpack 'check-vendor
          (lambda _
            (unless (file-exists? "vendor")
              (error "vendor/ missing -- run: guix shell -m manifest.scm -- cargo vendor vendor"))))
        (replace 'build
          (lambda _
            (setenv "HOME" (getcwd)) ; cargo wants a writable home
            (setenv "MINDE_BUILD_REVISION" #$%build-revision)
            ;; wayland-backend (libwayland-server.so.0), smithay's EGL
            ;; loader (libEGL.so.1 via glvnd) and friends are dlopened at
            ;; runtime, so the linker never records them and Guix's
            ;; automatic rpath doesn't cover them. Bake the search path in
            ;; -- a login session (SDDM) has no LD_LIBRARY_PATH to help.
            (setenv "RUSTFLAGS"
                    (string-join
                     (map (lambda (dir)
                            (string-append "-C link-arg=-Wl,-rpath=" dir "/lib"))
                          (list #$(this-package-input "wayland")
                                #$(this-package-input "libglvnd")
                                #$(this-package-input "mesa")
                                #$(this-package-input "libxkbcommon")
                                #$(this-package-input "libseat")
                                #$(this-package-input "libinput-minimal")
                                #$(this-package-input "eudev")))
                     " "))
            (mkdir-p ".cargo")
            (copy-file "guix/cargo-config.toml" ".cargo/config.toml")
            (invoke "cargo" "build" "--release" "--offline")))
        (replace 'check
          (lambda* (#:key tests? #:allow-other-keys)
            (when tests?
              (setenv "GUILE_AUTO_COMPILE" "0")
              (for-each
               (lambda (test)
                 (invoke "guile" "--no-auto-compile" "-L" "scheme" test))
               '("tests/frames-test.scm"
                 "tests/next-pull-test.scm"
                 "tests/groups-test.scm"
                 "tests/layouts-test.scm"
                 "tests/portable-keymap-test.scm")))))
        (replace 'install
          (lambda* (#:key inputs #:allow-other-keys)
            (let* ((out #$output)
                   (bin (string-append out "/bin"))
                   (share (string-append out "/share/minde"))
                   (guile-site (string-append out "/share/guile/site/3.0"))
                   (doc-out (string-append out "/share/doc/minde"))
                   (sessions (string-append out "/share/wayland-sessions"))
                   (mesa #$(this-package-input "mesa")))
              (install-file "target/release/minde" bin)
              (copy-recursively "scheme" (string-append share "/scheme"))
              (copy-recursively "scheme/minde"
                                (string-append guile-site "/minde"))
              (install-file "scheme/default-config.scm" share)
              ;; Bytecode cache: mirrors `make compile-scheme` (Makefile) so
              ;; a packaged login loads compiled .go files instead of
              ;; autocompiling the module tree into ~/.cache/guile/ccache
              ;; on the compositor main thread before the first output is
              ;; usable (see TODO.md 1.1). default-config.scm is data read
              ;; by the validator, not code, and is skipped like the
              ;; Makefile target skips it.
              (let* ((ccache (string-append out "/lib/guile/3.0/site-ccache"))
                     (scheme-src (string-append share "/scheme")))
                (define (directory-entries dir)
                  (let ((port (opendir dir)))
                    (let loop ((entry (readdir port)) (acc '()))
                      (if (eof-object? entry)
                          (begin (closedir port) (reverse acc))
                          (loop (readdir port)
                                (if (member entry '("." ".."))
                                    acc
                                    (cons entry acc)))))))
                (define (scheme-files dir prefix)
                  ;; Recursively list *.scm files under DIR, as paths
                  ;; relative to PREFIX's starting point (mirrors the
                  ;; Makefile's `find scheme -name '*.scm'`).
                  (append-map
                   (lambda (name)
                     (let* ((path (string-append dir "/" name))
                            (rel (if (string-null? prefix)
                                     name
                                     (string-append prefix "/" name))))
                       (cond
                        ((eq? 'directory (stat:type (stat path)))
                         (scheme-files path rel))
                        ((string-suffix? ".scm" name)
                         (list rel))
                        (else '()))))
                   (directory-entries dir)))
                (setenv "GUILE_AUTO_COMPILE" "0")
                (for-each
                 (lambda (relpath)
                   (unless (string=? relpath "default-config.scm")
                     (let* ((src (string-append scheme-src "/" relpath))
                            (go (string-append ccache "/"
                                                (string-drop-right relpath 4)
                                                ".go"))
                            (warn (if (string-index relpath #\/) '() '("-W0"))))
                       (mkdir-p (dirname go))
                       (apply invoke "guild" "compile" "-L" scheme-src
                              "-o" go (append warn (list src))))))
                 (scheme-files scheme-src "")))
              (mkdir-p doc-out)
              (for-each
               (lambda (file) (install-file file doc-out))
               '("README.md" "CHANGELOG.md" "CONTRIBUTING.md" "SECURITY.md"
                 "SUPPORT.md" "NOTICE" "COPYING"))
              (copy-recursively "LICENSES" (string-append doc-out "/LICENSES"))
              (copy-recursively "doc" (string-append doc-out "/doc"))
              (copy-recursively "release" (string-append doc-out "/release"))
              ;; Session wrapper: environment for a bare-TTY login.
              (mkdir-p sessions)
              (call-with-output-file (string-append bin "/minde-session")
                (lambda (port)
                  (format port "#!~a/bin/sh
# minde login session wrapper.
export XDG_CURRENT_DESKTOP=minde
export MINDE_SCHEME_DIR=~a/scheme
export GUILE_LOAD_PATH=~a:${GUILE_LOAD_PATH:-}
# Compiled bytecode cache installed above (mirrors `make compile-scheme`):
# load .go files instead of autocompiling the module tree into
# ~/.cache/guile/ccache on the compositor main thread before the first
# output is usable, and never recompile on a store mtime mismatch.
export GUILE_LOAD_COMPILED_PATH=~a:${GUILE_LOAD_COMPILED_PATH:-}
export GUILE_AUTO_COMPILE=0
# EGL vendor discovery on Guix (glvnd needs pointing at mesa).
export __EGL_VENDOR_LIBRARY_DIRS=~a/share/glvnd/egl_vendor.d
if [ ! -f \"$HOME/.config/minde/init.scm\" ]; then
  export MINDE_INIT=~a/scheme/init.scm
fi
# Keep full compositor output somewhere readable (SDDM's session log
# tends to lose stdout); panics additionally land in crash.log via the
# binary's own panic hook.
LOGDIR=\"${XDG_STATE_HOME:-$HOME/.local/state}/minde\"
mkdir -p \"$LOGDIR\"
# Keep exactly the current and previous session; overwrite the previous
# generation at login so stale logs do not accumulate.
if [ -f \"$LOGDIR/session.log\" ]; then
  mv -f \"$LOGDIR/session.log\" \"$LOGDIR/session.previous.log\"
fi
export RUST_BACKTRACE=1
exec ~a/minde --tty \"$@\" > \"$LOGDIR/session.log\" 2>&1
"
                          #$(this-package-input "bash-minimal")
                          share guile-site
                          (string-append out "/lib/guile/3.0/site-ccache")
                          mesa share bin)))
              (chmod (string-append bin "/minde-session") #o755)
              ;; REPL-socket helper scripts (used by prompt/message
              ;; workflows spawned from bindings). Pin their guile.
              ;; mindectl is the only one that invokes guile directly (via
              ;; the single GUILE=${MINDE_GUILE:-guile} line); minde-cmd
              ;; and minde-msg delegate to it, so this substitute* is a
              ;; no-op for those two (harmless: substitute* does not error
              ;; on zero matches) but keeps all three scripts pinned the
              ;; same way if that ever changes.
              (for-each
               (lambda (script)
                 (let ((dest (string-append bin "/" script)))
                   (copy-file (string-append "scripts/" script) dest)
                   (substitute* dest
                     (("^GUILE=\\$\\{MINDE_GUILE:-guile\\}")
                      (string-append "GUILE=" #$(this-package-input "guile")
                                     "/bin/guile")))
                   (chmod dest #o755)))
               '("minde-cmd" "minde-msg" "mindectl"))
              (call-with-output-file (string-append sessions "/minde.desktop")
                (lambda (port)
                  (display "[Desktop Entry]
Name=minde
Comment=StumpWM-style Guile Scheme Wayland compositor
Exec=minde-session
Type=Application
" port)))))))))
  (native-inputs
   (list rust
         (list rust "cargo")
         pkg-config
         guile-3.0))
  (inputs
   (list bash-minimal
         guile-3.0
         wayland
         libxkbcommon
         libinput-minimal
         eudev
         libseat
         mesa
         libglvnd))
  ;; smithay execs "Xwayland" from PATH at runtime; propagation puts it
  ;; into the same profile as minde (system profile via SDDM).
  (propagated-inputs
   (list xorg-server-xwayland))
  (home-page "https://github.com/9s-l-s9/minde")
  (synopsis "StumpWM-style Wayland compositor scripted in Guile Scheme")
  (description
   "minde is a Wayland compositor built on Smithay whose entire policy
layer (frames, groups, keybindings) lives in embedded Guile Scheme, with a
live REPL for runtime reconfiguration.")
  (license license:gpl3+))

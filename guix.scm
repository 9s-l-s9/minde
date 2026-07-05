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
  (version "0.1.0")
  (source (local-file %source-dir "minde-source"
                      #:recursive? #t
                      #:select? source-select?))
  (build-system gnu-build-system)
  (arguments
   (list
    #:tests? #f ; scheme tests need only guile; run via `make check` equiv below
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
            ;; Exactly what `cargo vendor` prints (the git source key must
            ;; carry the ?rev= qualifier to match Cargo.lock).
            (call-with-output-file ".cargo/config.toml"
              (lambda (port)
                (display "\
[source.crates-io]
replace-with = \"vendored-sources\"

[source.\"git+https://github.com/Smithay/Smithay?rev=3021f619e2ae4dab8bfb1e21f3f210923b9b6582\"]
git = \"https://github.com/Smithay/Smithay\"
rev = \"3021f619e2ae4dab8bfb1e21f3f210923b9b6582\"
replace-with = \"vendored-sources\"

[source.vendored-sources]
directory = \"vendor\"
" port)))
            (invoke "cargo" "build" "--release" "--offline")))
        (replace 'check
          (lambda* (#:key tests? #:allow-other-keys)
            (when tests?
              (invoke "guile" "-L" "scheme" "tests/frames-test.scm")
              (invoke "guile" "-L" "scheme" "tests/next-pull-test.scm")
              (invoke "guile" "-L" "scheme" "tests/groups-test.scm")
              (invoke "guile" "-L" "scheme" "tests/keys-test.scm")
              (invoke "guile" "-L" "scheme" "tests/layouts-test.scm"))))
        (replace 'install
          (lambda* (#:key inputs #:allow-other-keys)
            (let* ((out #$output)
                   (bin (string-append out "/bin"))
                   (share (string-append out "/share/minde"))
                   (sessions (string-append out "/share/wayland-sessions"))
                   (mesa #$(this-package-input "mesa")))
              (install-file "target/release/minde" bin)
              (copy-recursively "scheme" (string-append share "/scheme"))
              ;; Session wrapper: environment for a bare-TTY login.
              (mkdir-p sessions)
              (call-with-output-file (string-append bin "/minde-session")
                (lambda (port)
                  (format port "#!~a/bin/sh
# minde login session wrapper.
export XDG_CURRENT_DESKTOP=minde
export MINDE_SCHEME_DIR=~a/scheme
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
                          share mesa share bin)))
              (chmod (string-append bin "/minde-session") #o755)
              ;; REPL-socket helper scripts (used by prompt/message
              ;; workflows spawned from bindings). Pin their guile.
              (for-each
               (lambda (script)
                 (let ((dest (string-append bin "/" script)))
                   (copy-file (string-append "scripts/" script) dest)
                   (substitute* dest
                     (("^    guile -q")
                      (string-append "    " #$(this-package-input "guile")
                                     "/bin/guile -q"))
                     (("^exec guile")
                      (string-append "exec " #$(this-package-input "guile")
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

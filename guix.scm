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
             ((guix licenses) #:prefix license:)
             (gnu packages base)
             (gnu packages rust)
             (gnu packages pkg-config)
             (gnu packages guile)
             (gnu packages freedesktop)
             (gnu packages xdisorg)
             (gnu packages gl)
             (gnu packages linux)
             (gnu packages bash))

(define %source-dir (dirname (current-filename)))

(define (source-select? file stat)
  ;; Everything except build artifacts and VCS metadata. vendor/ IS
  ;; included -- it's the offline crate mirror.
  (let ((name (basename file)))
    (not (member name '("target" ".git")))))

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
        (add-after 'unpack 'check-vendor
          (lambda _
            (unless (file-exists? "vendor")
              (error "vendor/ missing -- run: guix shell -m manifest.scm -- cargo vendor vendor"))))
        (replace 'build
          (lambda _
            (setenv "HOME" (getcwd)) ; cargo wants a writable home
            (mkdir-p ".cargo")
            (call-with-output-file ".cargo/config.toml"
              (lambda (port)
                (display "[source.crates-io]\nreplace-with = \"vendored\"\n\n[source.\"git+https://github.com/Smithay/Smithay\"]\ngit = \"https://github.com/Smithay/Smithay\"\nreplace-with = \"vendored\"\n\n[source.vendored]\ndirectory = \"vendor\"\n" port)))
            (invoke "cargo" "build" "--release" "--offline")))
        (replace 'check
          (lambda* (#:key tests? #:allow-other-keys)
            (when tests?
              (invoke "guile" "-L" "scheme" "tests/frames-test.scm")
              (invoke "guile" "-L" "scheme" "tests/groups-test.scm")
              (invoke "guile" "-L" "scheme" "tests/keys-test.scm"))))
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
: \"${XKB_DEFAULT_LAYOUT:=de}\"
: \"${XKB_DEFAULT_VARIANT:=bone}\"
export XKB_DEFAULT_LAYOUT XKB_DEFAULT_VARIANT
export XDG_CURRENT_DESKTOP=minde
export MINDE_SCHEME_DIR=~a/scheme
# EGL vendor discovery on Guix (glvnd needs pointing at mesa).
export __EGL_VENDOR_LIBRARY_DIRS=~a/share/glvnd/egl_vendor.d
if [ ! -f \"$HOME/.config/minde/init.scm\" ]; then
  export MINDE_INIT=~a/scheme/init.scm
fi
exec ~a/minde --tty \"$@\"
"
                          #$(this-package-input "bash-minimal")
                          share mesa share bin)))
              (chmod (string-append bin "/minde-session") #o755)
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
  (home-page "https://github.com/s-l-s/minde")
  (synopsis "StumpWM-style Wayland compositor scripted in Guile Scheme")
  (description
   "minde is a Wayland compositor built on Smithay whose entire policy
layer (frames, groups, keybindings) lives in embedded Guile Scheme, with a
live REPL for runtime reconfiguration.")
  (license license:gpl3+))

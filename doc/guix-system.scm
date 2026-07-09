;;; Snippet: making minde selectable in SDDM on Guix System.
;;;
;;; SDDM lists Wayland sessions from share/wayland-sessions/ of packages in
;;; the system profile. Add the minde package (built from this repo's
;;; guix.scm) to your operating-system's packages, then
;;; `sudo guix system reconfigure /etc/config.scm`.

;; In /etc/config.scm (or wherever your system config lives):

;; (define minde
;;   ;; Load the package definition straight from the checkout. Remember the
;;   ;; one-time `guix shell -m manifest.scm -- cargo vendor vendor` step in
;;   ;; the repo before reconfiguring (see guix.scm header).
;;   (load "/path/to/minde/guix.scm"))

;; For a release archive, set MINDE_SOURCE_ARCHIVE, MINDE_VERSION, and
;; MINDE_BUILD_REVISION, then load guix/release.scm instead. See
;; doc/releasing.md for the checked archive-only workflow.

;; (operating-system
;;   ...
;;   (packages (cons* minde
;;                    ;; ... your existing packages ...
;;                    %base-packages)))

;; After reconfigure, the SDDM greeter's session dropdown gains a
;; "minde" (Wayland) entry next to your X11 StumpWM session. Your
;; StumpWM setup is untouched -- both sessions coexist and you pick at
;; login. Per-user Scheme config: ~/.config/minde/init.scm.

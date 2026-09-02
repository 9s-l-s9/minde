;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Stable, curated window-management API.

(define-module (minde windows)
  #:use-module (minde compositor frames)
  #:use-module (minde compositor rust)
  #:export (window-geometry)
  #:re-export (focused-window-id
               current-frame-window
               all-window-ids
               window-title
               window-app-id
               window-floating?
               window-number
               focus-window-by-id!
               focus-next-window!
               focus-previous-window!
               select-window-by-number!
               pull-window-by-number!
               pull-window-by-id!
               kill-current-window!
               close-current-window!
               float-window!
               unfloat-window!
               toggle-always-on-top!
               toggle-always-show!
               rename-window!
               mark-window-toggle!
               marked-windows
               clear-marks!
               pull-marked!
               urgent-windows
               clear-urgent!))

(define (window-geometry id)
  "Return visible window ID's (x y width height) in global logical
coordinates, or #f when the window is unknown, hidden, or unmapped.  The
coordinates are directly usable with wm-warp-pointer; output reservations and
configured gaps are already reflected in the rectangle."
  (rust-call-if-bound 'wm-window-geometry id))

;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Stable, curated window-management API.

(define-module (minde windows)
  #:use-module (minde frames)
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

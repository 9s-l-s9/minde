;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Shared lock redaction for live events and journal replay.

(define-module (minde event-policy)
  #:export (event-redact-args))

(define (event-redact-args name args)
  "Returns the locked-session payload for NAME, or #f to suppress it.
Preserves known state notifications, blanks titles and application IDs,
and suppresses messages and unknown events."
  (case name
    ((new-window window-title-changed)
     ;; Even a partial title payload must not retain its content fields.
     (and (pair? args) (list (car args) "" "")))
    ((focus-window focus-frame focus-group urgent-window destroy-window
      session-lock session-unlock automation-result window-geometry
      window-geometry-settled)
     args)
    (else #f)))

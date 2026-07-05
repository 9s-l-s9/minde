;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Stable text status API. Structured IPC status is introduced later.

(define-module (minde status)
  #:use-module (minde groups)
  #:export (current-status-text))

(define (current-status-text)
  "Returns a one-line summary suitable for an external bar such as eww."
  (status-line))

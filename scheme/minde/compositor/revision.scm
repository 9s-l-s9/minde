;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Internal invalidation shared by policy and optional control inspection.
(define-module (minde compositor revision)
  #:export (policy-revision note-policy-change!))

(define %revision 0)
(define (policy-revision) %revision)
(define (note-policy-change!) (set! %revision (+ %revision 1)))

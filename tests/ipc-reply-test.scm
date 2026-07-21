;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Self-describing IPC reply envelope: error payloads and the
;;; writable-data guarantee (scheme/ipc-reply.scm).

(use-modules (system vm trace)
             (minde commands)
             (minde command-catalog))

;; wm-log is a Rust gsubr at runtime; stub it so the envelope loads headlessly.
(define %log '())
(define (wm-log message) (set! %log (cons message %log)) #t)

(load-from-path "ipc-reply.scm")

(define failures 0)
(define (check description value)
  (unless value
    (set! failures (+ failures 1))
    (format #t "FAIL - ~a~%" description)))

;; A reply string must always be exactly one datum that reads back cleanly.
(define (round-trip reply)
  (call-with-input-string reply
    (lambda (port)
      (let ((datum (read port)))
        (check "reply is a single datum" (eof-object? (read port)))
        datum))))

;; --- Success replies -------------------------------------------------------

(let ((datum (round-trip (minde-ipc-eval "(list 1 2 (quote sym) \"str\")"))))
  (check "success is tagged ok" (eq? (car datum) 'ok))
  (check "success carries the result" (equal? (cadr datum) '(1 2 sym "str"))))

;; --- Error replies carry key, args, message and bounded backtrace ----------

(let ((datum (round-trip (minde-ipc-eval "(error \"boom\" 42)"))))
  (check "error is tagged error" (eq? (car datum) 'error))
  (check "error has five elements" (= (length datum) 5))
  (check "error key is a symbol" (symbol? (cadr datum)))
  (check "error message is a non-empty string"
         (and (string? (list-ref datum 3))
              (not (string-null? (list-ref datum 3)))))
  (check "error message mentions the condition"
         (string-contains (list-ref datum 3) "boom"))
  (check "error backtrace is a string" (string? (list-ref datum 4)))
  (check "error backtrace is bounded"
         (<= (string-length (list-ref datum 4)) (+ %ipc-backtrace-max-chars 32))))

;; A read error (more than one datum) is also reported, not crashed on.
(let ((datum (round-trip (minde-ipc-eval "1 2"))))
  (check "surplus input is an error" (eq? (car datum) 'error))
  (check "surplus-input message present" (string? (list-ref datum 3))))

;; --- Writable-data guarantee ----------------------------------------------

;; A value that prints as #<...> must never leak into an ok reply.
(let ((datum (round-trip (minde-ipc-eval "(current-output-port)"))))
  (check "unreadable result becomes an error" (eq? (car datum) 'error))
  (check "unreadable result is flagged" (eq? (cadr datum) 'unreadable-result)))

;; The guarantee covers error replies too: throw ARGS carrying a live object
;; (here a wrong-type-arg irritant that prints as #<procedure ...>) must be
;; sanitized, keeping the whole error datum re-readable.
(let ((datum (round-trip (minde-ipc-eval "(car car)"))))
  (check "error with live irritant is still an error" (eq? (car datum) 'error))
  ;; The datum must survive a second write/read cycle unchanged: live objects
  ;; were replaced by strings, so nothing prints as a raw #<...> token.
  (check "error datum with live irritant round-trips"
         (equal? datum
                 (call-with-input-string
                  (call-with-output-string (lambda (p) (write datum p)))
                  read))))

;; ipc-ok-reply classifies readable vs unreadable values directly.
(check "plain data is ok" (eq? (car (ipc-ok-reply '(a 1 "b"))) 'ok))
(check "procedure is rejected" (eq? (car (ipc-ok-reply car)) 'error))

;; Every ok reply round-trips through write/read and equals the input.
(for-each
 (lambda (value)
   (let* ((reply (ipc-ok-reply value))
          (text (call-with-output-string (lambda (p) (write reply p))))
          (back (call-with-input-string text read)))
     (check (format #f "round-trips: ~s" value)
            (and (eq? (car back) 'ok) (equal? (cadr back) value)))))
 (list '() 'sym "string" 42 -1.5 #t #f '(1 (2 3) "x") '((a . 1) (b . 2))))

;; --- Catalog surface: metadata queries return writable data ----------------
;; Iterate the built-in command catalog and verify that introspecting each
;; command through the real IPC reply path yields ok, re-readable data.

(register-builtin-command-schemas!)
(let ((names (command-names)))
  (check "catalog is non-empty" (> (length names) 0))
  (for-each
   (lambda (name)
     (let* ((expr (format #f "(map (lambda (c) (list (command-summary c) (command-category c) (command-arguments c))) (list (command-ref '~a)))" name))
            (datum (round-trip (minde-ipc-eval expr))))
       (check (format #f "catalog metadata ok for ~a" name)
              (eq? (car datum) 'ok))))
   names))

(if (zero? failures)
    (format #t "ipc-reply-test: all checks passed~%")
    (begin
      (format #t "ipc-reply-test: ~a failure(s)~%" failures)
      (exit 1)))

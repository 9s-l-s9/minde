;;; callbacks-test.scm -- the compositor callback contract stays in sync
;;; with the Rust `Hook` table, and the registry checks arity.
;;;
;;; Run with: guile -L scheme tests/callbacks-test.scm

(use-modules (srfi srfi-1) (ice-9 rdelim) (ice-9 regex)
             (minde compositor callbacks))

(define failures 0)
(define (check name ok?)
  (if ok?
      (format #t "ok - ~a~%" name)
      (begin (set! failures (+ failures 1))
             (format #t "FAIL - ~a~%" name))))

;; --- Every Hook::new("...") name in src/guile/mod.rs has an arity entry --
(define rust-hooks
  (call-with-input-file "src/guile/mod.rs"
    (lambda (port)
      (let loop ((names '()))
        (let ((line (read-line port)))
          (if (eof-object? line)
              (reverse names)
              (let ((m (string-match "Hook::new\\(\"([^\"]+)\"\\)" line)))
                (loop (if m (cons (string->symbol (match:substring m 1)) names)
                          names)))))))))

(define registry (map car (compositor-callback-arities)))
(check "rust hook table parsed" (> (length rust-hooks) 10))
(check "every Rust hook has a registry entry"
       (every (lambda (n) (memq n registry)) rust-hooks))
(check "every registry entry is a Rust hook"
       (every (lambda (n) (memq n rust-hooks)) registry))

;; --- Arity checking ---------------------------------------------------
(define (policy) (resolve-module '(guile-user)))
(define-compositor-callback! 'handle-window-unmap! (lambda (id) id))
(check "checked definition installs the procedure"
       (procedure? (module-ref (policy) 'handle-window-unmap!)))
(check "definition with too few parameters is rejected"
       (catch #t
         (lambda () (define-compositor-callback! 'handle-window-map! (lambda (id) id)) #f)
         (lambda _ #t)))
(check "definition of an unknown name is rejected"
       (catch #t
         (lambda () (define-compositor-callback! 'no-such-hook! (lambda () #t)) #f)
         (lambda _ #t)))
(check "rest-argument procedures are accepted"
       (begin (define-compositor-callback! 'wm-handle-key (lambda (a b c . rest) #t)) #t))

;; --- Status report ------------------------------------------------------
(module-define! (policy) 'handle-timer! (lambda (a b) #t))
(define problems (check-compositor-callbacks!))
(check "arity mismatch is reported"
       (member '(handle-timer! 1 arity-mismatch) problems))
(check "unbound callbacks are reported as unbound"
       (any (lambda (p) (eq? (caddr p) 'unbound)) problems))
(check "installed callbacks are ok"
       (equal? (assq-ref (map (lambda (e) (cons (car e) (cddr e)))
                              (compositor-callbacks))
                         'handle-window-unmap!)
               '(ok)))

(if (zero? failures)
    (begin (display "callbacks-test: all checks passed\n") (exit 0))
    (begin (format #t "callbacks-test: ~a failure(s)~%" failures) (exit 1)))

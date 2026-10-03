;;; SPDX-License-Identifier: GPL-3.0-or-later
(use-modules (minde control-client) (minde control-json) (srfi srfi-13))
(define failures 0)
(define (check name value)
  (unless value (set! failures (+ failures 1)) (format #t "FAIL: ~a~%" name)))
(define (throws? thunk) (catch #t (lambda () (thunk) #f) (lambda _ #t)))

(check "read exactly one datum" (equal? (read-one-datum "((a . 1))") '((a . 1))))
(check "empty reader input rejected" (throws? (lambda () (read-one-datum ""))))
(check "surplus reader input rejected" (throws? (lambda () (read-one-datum "1 2"))))
(define changed? #f)
(with-fluids ((read-eval? #t))
  (check "reader eval disabled regardless of inherited setting"
         (throws? (lambda () (read-one-datum "#.(set! changed? #t)")))))
(check "reader did not execute input" (not changed?))

(let* ((name "name) (error \"injected\") (")
       (value "text\" (error \"injected\")")
       (arguments (call-with-output-string (lambda (p) (write `((text . ,value)) p))))
       (code (read-one-datum (build-control-expression
                             "call" (list name arguments)
                             '(("--session" . "s\"quoted") ("--revision" . 7) ("--request-id" . "s:1"))))))
  (check "action name is quoted data" (equal? (cadr code) `(quote ,(string->symbol name))))
  (check "named arguments are quoted data" (equal? (caddr code) `(quote ((text . ,value)))))
  (check "session is a literal string" (equal? (list-ref code 4) "s\"quoted")))
(check "raw eval retains source" (equal? (build-control-expression "eval" '("(+ 1 2)") '()) "(+ 1 2)"))
(check "unknown command rejected" (throws? (lambda () (build-control-expression "unknown" '() '()))))
(check "invalid cursor rejected" (throws? (lambda () (build-control-expression "events-since" '("-1") '()))))
(check "raw list is tagged, never guessed to be an object"
       (string=? (control-json '((a . 1)) 'raw) "{\"$scheme\":\"((a . 1))\"}"))
(check "empty raw list retains Scheme identity"
       (string=? (control-json '() 'raw) "{\"$scheme\":\"()\"}"))
(check "raw unspecified remains explicit"
       (string=? (control-json '(unspecified) 'raw) "{\"$scheme\":\"(unspecified)\"}"))
(check "raw rationals retain exactness"
       (string=? (control-json 1/3 'raw) "{\"$scheme\":\"1/3\"}"))
(check "raw symbols and strings differ"
       (not (equal? (control-json 'a 'raw) (control-json "a" 'raw))))
(check "known object and vector schemas become JSON objects and arrays"
       (string=? (control-json '((windows . #()) (focused-window . null)) 'desktop)
                 "{\"windows\":[],\"focused-window\":null}"))
(check "legacy effect metadata remains unknown"
       (string=? (control-json '((actions . (((effects . unknown))))) 'capabilities)
                 "{\"actions\":[{\"effects\":\"unknown\"}]}"))
(check "action missing value is JSON null"
       (string=? (control-json '((status . pending) (value . null)) 'action)
                 "{\"status\":\"pending\",\"value\":null}"))
(check "layout tree is a declared object"
       (string-contains (control-json '((groups . #(((trees . #(((head . 0) (scheme . "(leaf)")))))))) 'desktop)
                        "\"trees\":[{\"head\":0,\"scheme\":\"(leaf)\"}]"))
(check "JSON escaping preserves control characters and Unicode"
       (string=? (control-json "λ\n\"\\" 'raw) "\"λ\\n\\\"\\\\\""))

(if (zero? failures) (display "control-client-test: all checks passed\n") (exit 1))

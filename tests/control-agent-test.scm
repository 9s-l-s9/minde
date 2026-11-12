;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Offline selector contracts, independent of model accuracy or credentials.
(use-modules (minde agent) (minde commands) (minde command-catalog)
             (minde control) (minde control-json) (json)
             (ice-9 rdelim) (ice-9 binary-ports) (rnrs bytevectors)
             (srfi srfi-1) (srfi srfi-13) (srfi srfi-64))
(test-begin "control-agent")
(define corpus (read-json "tests/fixtures/control-agent/corpus.json"))
(define snapshot (ref corpus "snapshot"))
(define catalog (ref corpus "catalog"))
(define candidates (candidates-from snapshot catalog %supported))
(define (replace value key item) (acons key item (alist-delete key value)))
(define (code thunk)
  (catch 'agent-error (lambda () (thunk) #f) (lambda (_ name) name)))
(define (response)
  (object (cons "model" %model)
          (cons "answers" (object
                          (cons "selection"
                                (object (cons "type" "choice") (cons "choice" "c0") (cons "confidence" 1)
                                        (cons "probabilities"
                                              (map (lambda (key) (cons key (if (equal? key "c0") 1 0)))
                                                   (append '("none" "ambiguous") (map (lambda (c) (ref c "choice-id")) candidates))))))))))
(define (change-answer key value)
  (let ((r (response)))
    (replace r "answers" (object (cons "selection" (replace (ref (ref r "answers") "selection") key value))))))
(let ((report (evaluate corpus)))
  (test-equal "synthetic baseline" 10 (ref (ref report "metrics") "correct"))
  (test-equal "no wrong targets" 0 (ref (ref report "metrics") "wrong-target"))
  (test-equal "abstentions" 5 (ref (ref report "metrics") "abstained"))
  (test-equal "offline" #f (ref report "provider-contact-attempted"))
  (test-equal "evaluation never executes" "disabled" (ref report "execution")))
(test-equal "discovery limits candidates" 2 (length (candidates-from snapshot catalog '("switch-group!"))))
(test-equal "repeated allow entries are a set" 2
  (length (candidates-from snapshot catalog '("switch-group!" "switch-group!"))))
(for-each
 (lambda (bad)
   (test-equal "malformed capability catalog" "unsupported-catalog"
     (code (lambda () (discover-catalog (lambda _ bad) %supported)))))
 (list #() '(("schema-version" . #t) ("actions" . #()))
       '(("schema-version" . 1) ("actions" . #("bad")))
       '(("schema-version" . 1) ("actions" . #((("name" . 1)))))))
(test-equal "schema version is not boolean" "unsupported-snapshot"
  (code (lambda () (candidates-from (replace snapshot "schema-version" #t) catalog %supported))))
(register-builtin-command-schemas!)
(register-control-commands!)
(test-equal "live registry JSON contract" (length candidates)
  (length (candidates-from snapshot
                            (vector (load-json (control-json (describe-action 'focus-window!) 'description))
                                    (load-json (control-json (describe-action 'switch-group!) 'description)))
                            %supported)))
(let ((window (vector-ref (ref snapshot "windows") 0)))
  (test-equal "bounded candidates" "too-many-candidates"
    (code (lambda ()
            (candidates-from (replace snapshot "windows" (list->vector (map (lambda (id) (replace window "id" id)) (iota 254 1))))
                             catalog '("focus-window!")))))
  (test-equal "duplicate targets" "duplicate-desktop-target"
    (code (lambda () (candidates-from (replace snapshot "windows" (vector window window)) catalog %supported)))))
(test-equal "redacted desktop" "desktop-unavailable"
  (code (lambda () (candidates-from (replace snapshot "redacted" #t) catalog %supported))))
(let* ((question (ref (ref (provider-request "focus browser" candidates) "questions") "selection"))
       (facts (ref (ref (ref question "criteria") "c0") "untrusted_descriptive_data")))
  (test-equal "titles remain data" 42 (ref facts "window-id"))
  (test-assert "no provider-supplied arguments" (not (string-contains (scm->json-string question) "arguments"))))
(test-equal "valid provider response" "c0" (ref (parse-provider-response (response) candidates) "choice"))
(for-each
 (lambda (entry)
   (test-equal "malformed provider response" "malformed-provider-response"
     (code (lambda () (parse-provider-response (change-answer (car entry) (cdr entry)) candidates)))))
 (list (cons "choice" "(wm-quit)") (cons "confidence" +nan.0) (cons "confidence" #t)
       (cons "probabilities" '(("c0" . 1)))))
(test-equal "wrong model" "malformed-provider-response"
  (code (lambda () (parse-provider-response (replace (response) "model" "unknown") candidates))))
(let* ((p (ref (ref (ref (response) "answers") "selection") "probabilities"))
       (tie (change-answer "probabilities" (replace (replace p "c0" 0.5) "c1" 0.5))))
  (test-equal "ties abstain" "ambiguous" (ref (parse-provider-response tie candidates) "choice")))
(for-each
 (lambda (confidence)
   (let ((result (select-action "focus browser" snapshot catalog #:provider "jev" #:threshold 0.9
                               #:selector (lambda _ (object (cons "choice" "c0") (cons "model" %model)
                                                            (cons "confidence" confidence))))))
     (test-equal "threshold" (if (< confidence 0.9) "abstained" "dry-run") (ref result "status"))))
 '(0.89 0.9 1))
(test-equal "unknown choice" "unknown-choice"
  (code (lambda () (select-action "focus browser" snapshot catalog
                                  #:selector (lambda _ '(("choice" . "evil") ("model" . "stub")))))))
(for-each (lambda (input) (test-assert "strict JSON" (code (lambda () (load-json input)))))
          (list "{\"x\":1,\"x\":2}" "{\"x\":NaN}" "{\"x\":1e999}" "[" "1 2"
                (make-string (+ 1 (* 1024 1024)) #\x)))
(test-equal "allowlist required" "execution-allowlist-required"
  (code (lambda () (select-action "focus browser" snapshot catalog #:execute? #t))))
(let ((calls '()))
  (define (call . args)
    (set! calls (append calls (list args)))
    (if (equal? (car args) "request")
        '(("session" . "synthetic-session") ("revision" . 7) ("request-id" . "r1"))
        '(("status" . "applied") ("session" . "synthetic-session") ("request-id" . "r1"))))
  (test-equal "guarded execution" "applied"
    (ref (ref (select-action "focus browser" snapshot catalog #:execute? #t #:allowed '("focus-window!") #:call call) "result") "status"))
  (test-equal "exact request" '("call" "focus-window!" "((window . 42))" "--session" "synthetic-session"
                               "--revision" "7" "--request-id" "r1") (cadr calls)))
(test-equal "stale receipt" "stale-state"
  (code (lambda () (execute-action (lambda _ '(("session" . "synthetic-session") ("revision" . 8) ("request-id" . "r2")))
                                    (car candidates) snapshot))))
(let ((result (execute-action
               (lambda args
                 (if (equal? (car args) "request")
                     '(("session" . "synthetic-session") ("revision" . 7) ("request-id" . "r1"))
                     (throw 'agent-error "transport-outcome-unknown"))) (car candidates) snapshot)))
  (test-equal "uncertain execution retains receipt" "r1" (ref result "request-id"))
  (test-equal "uncertain execution not retried" "outcome-unknown" (ref result "status")))
(let ((calls 0))
  (test-equal "local transport" "c0"
    (ref (provider-select "focus browser" candidates 0.8 "local" #:endpoint "http://127.0.0.1:8123/select"
                          #:transport (lambda (url payload key timeout local?)
                                        (set! calls (+ calls 1))
                                        (test-equal "no local credentials" #f key)
                                        (test-equal "provider payload" %model (ref (load-json payload) "model"))
                                        (test-equal "bounded transport" 10 timeout)
                                        (response))) "choice"))
  (test-equal "no retries" 1 calls))
(for-each
 (lambda (endpoint)
   (test-equal "loopback only" "invalid-local-endpoint"
     (code (lambda () (provider-select "request" candidates 0.8 "local" #:endpoint endpoint)))))
 '("https://example.com" "http://evil.test" "http://user:pass@localhost" "http://localhost/#fragment"))
(define (http-case status body expected)
  (let ((listener (socket AF_INET SOCK_STREAM 0)) (child #f))
    (dynamic-wind
      (lambda () #t)
      (lambda ()
        (bind listener AF_INET INADDR_LOOPBACK 0)
        (listen listener 1)
        (force-output)
        (set! child (primitive-fork))
        (when (zero? child)
          (sigaction SIGPIPE SIG_IGN)
          (catch #t
            (lambda ()
              (unless (pair? (car (select (list listener) '() '() 5))) (error "no provider request"))
              (let ((connection (car (accept listener))))
                (read-line connection)
                (let loop ((length #f))
                  (let ((line (string-trim-right (read-line connection) #\return)))
                    (if (string-null? line)
                        (let ((payload (load-json (utf8->string (get-bytevector-n connection length)))))
                          (unless (equal? (ref payload "model") %model) (error "wrong model")))
                        (begin
                          (when (string-prefix-ci? "authorization:" line) (error "local credential leak"))
                          (loop (if (string-prefix-ci? "content-length:" line)
                                    (string->number (string-trim-both (substring line 15))) length))))))
                (format connection "HTTP/1.1 ~a Test\r\nContent-Length: ~a\r\nConnection: close\r\n\r\n~a"
                        status (bytevector-length (string->utf8 body)) body)
                (force-output connection) (close-port connection))
              (primitive-exit 0))
            (lambda _ (primitive-exit 1))))
        (let ((endpoint (format #f "http://127.0.0.1:~a/select" (sockaddr:port (getsockname listener)))))
          (test-equal "real HTTP transport" expected
            (catch 'agent-error
              (lambda () (ref (provider-select "focus browser" candidates 0.8 "local" #:endpoint endpoint) "choice"))
              (lambda (_ code) code))))
        (test-equal "HTTP request contract" 0 (status:exit-val (cdr (waitpid child))))
        (set! child #f))
      (lambda ()
        (when child (false-if-exception (kill child SIGKILL)) (false-if-exception (waitpid child)))
        (close-port listener)))))
(http-case 200 (scm->json-string (response)) "c0")
(http-case 302 (scm->json-string (response)) "provider-redirect")
(http-case 200 "{\"answers\":false}" "malformed-provider-response")
(let ((runner (test-runner-current)))
  (test-end "control-agent")
  (exit (zero? (+ (test-runner-fail-count runner) (test-runner-xpass-count runner)))))

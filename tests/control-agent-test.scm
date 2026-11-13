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
(define (with-window-field key value)
  (replace snapshot "windows"
           (list->vector
            (map (lambda (window)
                   (if (= (ref window "id") 45) (replace window key value) window))
                 (vector->list (ref snapshot "windows"))))))
(define (with-group-name value)
  (replace
   (replace snapshot "groups"
            (vector (vector-ref (ref snapshot "groups") 0) (object (cons "name" value))))
   "windows"
   (list->vector (map (lambda (window)
                       (if (equal? (ref window "group") "chat") (replace window "group" value) window))
                     (vector->list (ref snapshot "windows"))))))
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

;; Bound provider descriptions without changing local matching or targets.
(for-each
 (lambda (key limit)
   (for-each
    (lambda (size)
      (let* ((value (make-string size #\λ))
             (desktop (if (equal? key "group") (with-group-name value) (with-window-field key value)))
             (built (catch 'agent-error (lambda () (candidates-from desktop catalog %supported))
                           (lambda (_ error) error))))
        (test-assert "long descriptive fields do not invalidate the desktop" (list? built))
        (when (list? built)
          (test-equal "all candidates remain available" (length candidates) (length built))
          (test-equal "an unrelated browser remains selectable" 42
            (ref (ref (select-action "focus my browser" desktop catalog) "arguments") "window"))
          (test-equal "window IDs remain selectable despite long metadata" 45
            (ref (ref (select-action "focus window 45" desktop catalog) "arguments") "window"))
          (let* ((candidate (find (lambda (c) (equal? (ref (ref c "facts") "window-id") 45)) built))
                 (criteria (ref (ref (ref (provider-request "focus browser" built) "questions") "selection") "criteria"))
                 (description (ref (ref criteria (ref candidate "choice-id")) "untrusted_descriptive_data"))
                 (shortened (if (> size limit) (string-append (substring value 0 (- limit 1)) "…") value)))
            (test-equal "local metadata retains the original string" value (ref (ref candidate "facts") key))
            (test-equal "provider description is bounded at character boundaries" shortened (ref description key))
            (test-equal "bounded Unicode descriptions survive JSON" shortened
              (ref (load-json (scm->json-string description)) key))
            (when (equal? key "group")
              (let ((group (find (lambda (c) (equal? (ref (ref c "arguments") "group") value)) built)))
                (test-equal "provider group descriptions use the same bound" shortened
                  (ref (ref (ref criteria (ref group "choice-id")) "untrusted_descriptive_data") "group")))))
          (let* ((prefix (if (equal? key "group") "switch to group " "focus "))
                 (selected (select-action (string-append prefix value) desktop catalog)))
            (test-equal "full names remain exact local matches" "dry-run" (ref selected "status"))
            (test-equal "selection preserves the exact action target" (if (equal? key "group") value 45)
              (ref (ref selected "arguments") (if (equal? key "group") "group" "window")))
            (when (> size limit)
              (test-equal "a shortened prefix cannot alias a longer target" "none"
                (ref (select-action (string-append prefix (substring value 0 limit)) desktop catalog) "reason")))))))
    (list limit (+ limit 1) (+ limit 88))))
 '("title" "app-id" "group") '(512 128 128))
(for-each
 (lambda (key)
   (for-each
    (lambda (bad)
      (test-equal "wrong descriptive types still fail closed" "invalid-desktop-fact"
        (code (lambda () (candidates-from (with-window-field key bad) catalog %supported)))))
    (list #f 42 #() '(("nested" . "text")))))
 '("title" "app-id" "group"))
(test-equal "wrong group target types still fail closed" "invalid-desktop-target"
  (code (lambda () (candidates-from (replace snapshot "groups" (vector (object (cons "name" #f)))) catalog %supported))))
(test-equal "user request length remains bounded" "invalid-desktop-fact"
  (code (lambda () (select-action (make-string 4097 #\x) snapshot catalog))))
(let* ((directory (mkdtemp (string-copy "/tmp/minde-agent-text-XXXXXX")))
       (desktop-file (string-append directory "/desktop.json"))
       (catalog-file (string-append directory "/catalog.json")))
  (dynamic-wind
    (lambda () #t)
    (lambda ()
      (call-with-output-file desktop-file
        (lambda (port) (display (scm->json-string (with-window-field "title" (make-string 600 #\文))) port)))
      (call-with-output-file catalog-file (lambda (port) (display (scm->json-string catalog) port)))
      (let* ((status #f)
             (output (call-with-output-string
                       (lambda (port)
                         (parameterize ((current-output-port port))
                           (set! status (agent-main (list "select" "focus my browser"
                                                        "--snapshot" desktop-file "--catalog" catalog-file)))))))
             (result (load-json output)))
        (test-equal "CLI accepts a desktop with a long unrelated title" 0 status)
        (test-equal "CLI keeps the requested target" 42 (ref (ref result "arguments") "window"))))
    (lambda ()
      (for-each (lambda (path) (when (file-exists? path) (delete-file path))) (list desktop-file catalog-file))
      (rmdir directory))))

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

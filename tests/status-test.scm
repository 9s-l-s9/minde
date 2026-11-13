;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Structured status schema and atomic publication tests.

(use-modules (ice-9 rdelim) (srfi srfi-1))

(define %focused #f)
(define (wm-place-window . arguments) #t)
(define (wm-focus-window id) (set! %focused id) #t)
(define (wm-clear-focus) (set! %focused #f) #t)
(define (wm-focus-rect . arguments) #t)
(define (wm-output-geometry) '(0 0 1280 720))
(define %output-queries 0)
(define (wm-outputs)
  (set! %output-queries (+ %output-queries 1))
  '((7 0 0 1280 720 "eDP-1")))
(define (wm-runtime-info) '("winit" "ready" 2 1234))
(define %logs '())
(define (wm-log message) (set! %logs (cons message %logs)) #t)

(use-modules (minde compositor frames)
             (minde groups)
             (minde status))

(define failures 0)
(define (check description value)
  (unless value
    (set! failures (+ failures 1))
    (format #t "FAIL - ~a~%" description)))

;; Resolve paths without touching a live session or a shared /tmp file.
(let* ((names '("XDG_RUNTIME_DIR" "TMPDIR" "MINDE_STATUS_PATH"))
       (saved (map getenv names))
       (fallback (format #f "/tmp/minde-~a" (geteuid)))
       (temporary (format #f "/custom tmp/minde-~a" (geteuid))))
  (define (environment! values)
    (for-each (lambda (name value) (if value (setenv name value) (unsetenv name)))
              names values))
  (dynamic-wind
    (lambda () #t)
    (lambda ()
      (for-each
       (lambda (case)
         (environment! (take case 3))
         (check (format #f "status path for ~s" (take case 3))
                (string=? (status-file-path) (list-ref case 3)))
         (check (format #f "legacy path for ~s" (take case 3))
                (string=? ((@@ (minde status) legacy-status-file-path))
                          (list-ref case 4))))
       (list
        (list #f #f #f (string-append fallback "/status.json") (string-append fallback "/minde-status"))
        (list "" "" "" (string-append fallback "/status.json") (string-append fallback "/minde-status"))
        (list #f "/custom tmp" #f (string-append temporary "/status.json") (string-append temporary "/minde-status"))
        (list "" "/custom tmp" "" (string-append temporary "/status.json") (string-append temporary "/minde-status"))
        (list "/configured runtime" "/custom tmp" #f "/configured runtime/minde/status.json" "/configured runtime/minde-status")
        (list "/configured runtime" "" "" "/configured runtime/minde/status.json" "/configured runtime/minde-status")
        (list #f "/custom tmp" "/override/status.json" "/override/status.json" (string-append temporary "/minde-status")))))
    (lambda () (environment! saved))))

(define test-directory
  (string-append "/tmp/minde-status-test-" (number->string (getpid))))
(define test-path (string-append test-directory "/status.json"))
(setenv "XDG_RUNTIME_DIR" test-directory)
(setenv "MINDE_STATUS_PATH" test-path)

(update-output-geometry! 0 0 1280 720)
(handle-window-map! 42 "private title" "secret.app")

(let ((json (current-state-json)))
  (check "schema version is present" (string-contains json "\"schema_version\":1"))
  (check "runtime backend is present" (string-contains json "\"backend\":\"winit\""))
  (check "Xwayland state is present" (string-contains json "\"xwayland\":\"ready\""))
  (check "group state is present" (string-contains json "\"focused_group\":\" I \""))
  (check "focused window is present" (string-contains json "\"id\":42"))
  (check "window title is present in live status" (string-contains json "private title"))
  (check "outputs are present" (string-contains json "\"name\":\"eDP-1\""))
  (check "layout is present" (string-contains json "\"layout\":{")))

(let ((redacted (current-state-json #:redact? #t)))
  (check "report state omits window title" (not (string-contains redacted "private title")))
  (check "report state omits app id" (not (string-contains redacted "secret.app")))
  (check "report state retains focused id" (string-contains redacted "\"id\":42")))

(check "JSON encoder escapes remaining control characters"
       (string=? ((@@ (minde status) json-string)
                  (string #\a (integer->char 8) #\b))
                 "\"a\\u0008b\""))

(for-each
 (lambda (value)
   (check "bulk JSON strings match the escaping writer"
          (string=? ((@@ (minde status) json-string) value)
                    (call-with-output-string
                     (lambda (port)
                       ((@@ (minde status) write-escaped-json-string) value port))))))
 (append (list "" "ordinary ASCII title" "Grüße 世界 🪟" "quote\"slash\\")
         (map (lambda (code) (string #\a (integer->char code) #\b))
              (iota 32))))

(set! %output-queries 0)
(let ((first-sequence (publish-status!)))
  (check "publication queries its output snapshot once" (= %output-queries 1))
  (check "status file is created" (file-exists? test-path))
  (check "unchanged status is not republished" (= first-sequence (publish-status!)))
  (call-with-input-file test-path
    (lambda (port)
      (let ((contents (read-string port)))
        (check "published file contains one JSON object"
               (and (string-prefix? "{" contents)
                    (string-contains contents "\"sequence\":1")))))))

(set-sync-hook! publish-status!)
(add-urgent-window! 42)
(check "urgency-only update republishes status" (= (publish-status!) 2))
(check "urgent window is in structured state"
       (string-contains (current-state-json) "\"urgent_windows\":[42]"))

(handle-window-title-change! 42 "changed title" "secret.app")
(check "title-only update republishes status" (= (publish-status!) 3))

;; With the compositor's timer wrapper available, a burst of publications
;; coalesces into one deferred write carrying the latest sequence.
(define %deferred '())
(define %defer? #t)
(define (wm-run-after ms thunk)
  (and %defer? (begin (set! %deferred (cons thunk %deferred)) #t)))
(define (run-deferred!)
  (let ((callbacks %deferred))
    (set! %deferred '())
    (for-each (lambda (callback) (callback)) callbacks)))
(define (published-sequence)
  (call-with-input-file test-path
    (lambda (port)
      (let ((contents (read-string port)))
        (cond ((string-contains contents "\"sequence\":5") 5)
              ((string-contains contents "\"sequence\":4") 4)
              (else 3))))))
(add-urgent-window! 43)
(check "deferred publication advances the sequence" (= (publish-status!) 4))
(add-urgent-window! 44)
(check "second change in the burst advances again" (= (publish-status!) 5))
(check "one timer is armed for the burst" (= (length %deferred) 1))
(check "files are untouched until the timer fires" (= (published-sequence) 3))
(run-deferred!)
(check "the deferred write carries the latest sequence" (= (published-sequence) 5))
(check "the deferred write carries the latest state"
       (call-with-input-file test-path
         (lambda (port) (string-contains (read-string port) "44"))))

;; An existing directory at a file destination makes rename fail even when
;; the test runs with elevated permissions. Repair that same destination and
;; retry without a desktop change, in both immediate and deferred modes.
(define (contents path) (call-with-input-file path read-string))
(for-each
 (lambda (spec)
   (let* ((defer? (car spec)) (blocked-name (cadr spec)) (newer? (caddr spec))
          (label (format #f "retry-~a-~a-~a" defer? blocked-name newer?))
          (directory (string-append test-directory "/" label))
          (json-path (string-append directory "/status.json"))
          (legacy-path (string-append directory "/minde-status"))
          (blocked (string-append directory "/" blocked-name))
          (logged (length %logs)))
     (mkdir directory #o700)
     (mkdir blocked #o700)
     (setenv "XDG_RUNTIME_DIR" directory)
     (setenv "MINDE_STATUS_PATH" json-path)
     (set! %defer? defer?)
     (update-window-title! 42 label "secret.app")
     (let ((sequence (publish-status!)))
       (run-deferred!)
       (check "write failure is logged and swallowed" (= (length %logs) (+ logged 1)))
       (check "failed write does not arm a retry loop" (null? %deferred))
       (check "retry while still blocked retains the sequence" (= (publish-status!) sequence))
       (run-deferred!)
       (check "a persistent failure is retried on publication" (= (length %logs) (+ logged 2)))
       (rmdir blocked)
       (check "unchanged retry retains the failed sequence" (= (publish-status!) sequence))
       (when newer?
         (set! label (string-append label "-newer"))
         (update-window-title! 42 label "secret.app")
         (check "newer pending state advances once" (= (publish-status!) (+ sequence 1)))
         (set! sequence (+ sequence 1)))
       (check "repeated pending publication keeps its sequence" (= (publish-status!) sequence))
       (when defer? (check "retry burst uses one timer" (= (length %deferred) 1)))
       (run-deferred!)
       (check "structured status recovers at the repaired path" (file-exists? json-path))
       (check "legacy status recovers at the repaired path" (file-exists? legacy-path))
       (when (file-exists? json-path)
         (check "recovery writes the latest state" (string-contains (contents json-path) label))
         (check "recovery writes the correct sequence"
                (string-contains (contents json-path) (format #f "\"sequence\":~a," sequence))))
       (when (file-exists? legacy-path)
         (check "legacy recovery writes the latest state" (string-contains (contents legacy-path) label)))
       (when (and (file-exists? json-path) (file-exists? legacy-path))
         (let ((inodes (map (lambda (path) (stat:ino (stat path))) (list json-path legacy-path))))
           (check "successful retry restores unchanged-state suppression" (= (publish-status!) sequence))
           (check "successful retry leaves no timer armed" (null? %deferred))
           (check "unchanged success does not replace either file"
                  (equal? inodes (map (lambda (path) (stat:ino (stat path))) (list json-path legacy-path)))))))
     (for-each
      (lambda (path)
        (when (file-exists? path) (delete-file path))
        (let ((temporary (string-append path ".tmp." (number->string (getpid)))))
          (when (file-exists? temporary) (delete-file temporary))))
      (list json-path legacy-path))
     (rmdir directory)))
 '((#f "status.json" #f) (#f "minde-status" #f)
   (#t "status.json" #f) (#t "minde-status" #f) (#t "minde-status" #t)))

;; Standalone publication uses the same private TMPDIR fallback as IPC,
;; including the compatibility file, for both absent and empty runtime values.
(let* ((temporary (string-append test-directory "/temporary files"))
       (private (format #f "~a/minde-~a" temporary (geteuid)))
       (json (string-append private "/status.json"))
       (legacy (string-append private "/minde-status")))
  (mkdir temporary #o700)
  (setenv "TMPDIR" temporary)
  (setenv "MINDE_STATUS_PATH" "")
  (set! %defer? #f)
  (for-each
   (lambda (runtime)
     (if runtime (setenv "XDG_RUNTIME_DIR" runtime) (unsetenv "XDG_RUNTIME_DIR"))
     (update-window-title! 42 (if runtime "empty-runtime" "absent-runtime") "test")
     (publish-status!)
     (for-each
      (lambda (path)
        (check "fallback publication uses the private directory" (file-exists? path))
        (when (file-exists? path)
          (check "fallback status file is private" (= (stat:perms (stat path)) #o600))
          (delete-file path)))
      (list json legacy)))
   '(#f ""))
  (check "fallback directory is private" (= (stat:perms (stat private)) #o700))
  (rmdir private)
  (rmdir temporary))

(when (file-exists? test-path) (delete-file test-path))
(let ((legacy-path (string-append test-directory "/minde-status")))
  (when (file-exists? legacy-path) (delete-file legacy-path)))
(when (file-exists? test-directory) (rmdir test-directory))

(if (zero? failures)
    (format #t "all status tests passed~%")
    (exit 1))

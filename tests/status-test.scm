;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Structured status schema and atomic publication tests.

(use-modules (ice-9 rdelim))

(define %focused #f)
(define (wm-place-window . arguments) #t)
(define (wm-focus-window id) (set! %focused id) #t)
(define (wm-clear-focus) (set! %focused #f) #t)
(define (wm-focus-rect . arguments) #t)
(define (wm-output-geometry) '(0 0 1280 720))
(define (wm-outputs) '((7 0 0 1280 720 "eDP-1")))
(define (wm-runtime-info) '("winit" "ready" 2 1234))
(define (wm-log . arguments) #t)

(use-modules (minde compositor frames)
             (minde groups)
             (minde status))

(define failures 0)
(define (check description value)
  (unless value
    (set! failures (+ failures 1))
    (format #t "FAIL - ~a~%" description)))

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

(let ((first-sequence (publish-status!)))
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
(define (wm-run-after ms thunk) (set! %deferred (cons thunk %deferred)) #t)
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
((car %deferred))
(check "the deferred write carries the latest sequence" (= (published-sequence) 5))
(check "the deferred write carries the latest state"
       (call-with-input-file test-path
         (lambda (port) (string-contains (read-string port) "44"))))

(when (file-exists? test-path) (delete-file test-path))
(let ((legacy-path (string-append test-directory "/minde-status")))
  (when (file-exists? legacy-path) (delete-file legacy-path)))
(when (file-exists? test-directory) (rmdir test-directory))

(if (zero? failures)
    (format #t "all status tests passed~%")
    (exit 1))

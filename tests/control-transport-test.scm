;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Real Unix sockets with deterministic syscall interruptions, no compositor.
(use-modules (minde control-client) (ice-9 textual-ports)
             (rnrs bytevectors) (srfi srfi-64))
(test-begin "control-transport")
(define client (resolve-module '(minde control-client)))
(define real-connect connect)
(define real-select select)
(define real-send send)
(define real-recv! recv!)
(define (now-ms)
  (quotient (* (get-internal-real-time) 1000) internal-time-units-per-second))
(define (system-failure errno)
  (throw 'system-error "injected syscall" "~A" (list (strerror errno)) (list errno)))
(define (with-syscalls replacements thunk)
  ;; Replace the variables actually referenced by compiled client code, and
  ;; restore them even when the request fails. The server was forked earlier.
  (let ((saved (map (lambda (entry)
                     (let ((var (module-variable client (car entry))))
                       (list var (variable-ref var) (cdr entry)))) replacements)))
    (dynamic-wind
      (lambda () (for-each (lambda (entry) (variable-set! (car entry) (caddr entry))) saved))
      thunk
      (lambda () (for-each (lambda (entry) (variable-set! (car entry) (cadr entry))) saved)))))
(define (interruptions procedure errors)
  (lambda args
    (if (null? errors) (apply procedure args)
        (let ((next (car errors)))
          (set! errors (cdr errors))
          (if next (system-failure next) (apply procedure args))))))
(define (always-fails errno)
  (lambda args (usleep 2000) (system-failure errno)))
(define request "(display \"λ, interrupted IPC\")")
(define response "(ok \"λ, complete\")")
(define (short-send sock bytes)
  (let* ((n (min 3 (bytevector-length bytes))) (part (make-bytevector n)))
    (bytevector-copy! bytes 0 part 0 n)
    (real-send sock part)))
(define (short-recv! sock bytes)
  (let* ((part (make-bytevector 3)) (n (real-recv! sock part)))
    (bytevector-copy! part 0 bytes 0 n)
    n))

(define* (exchange replacements #:key (full-queue? #f) (drain? #t) (timeout-ms 1000))
  (let* ((directory (mkdtemp (string-copy "/tmp/minde-transport-XXXXXX")))
         (path (string-append directory "/ipc"))
         (received (string-append directory "/request"))
         (listener (socket AF_UNIX SOCK_STREAM 0))
         (blocker #f) (server #f))
    (dynamic-wind
      (lambda () #t)
      (lambda ()
        (bind listener AF_UNIX path)
        (listen listener 0)
        (when full-queue?
          (set! blocker (socket AF_UNIX SOCK_STREAM 0))
          (real-connect blocker AF_UNIX path))
        (when drain?
          (force-output)
          (set! server (primitive-fork))
          (when (zero? server)
            (sigaction SIGPIPE SIG_IGN)
            (alarm 3)
            (catch #t
              (lambda ()
                (when blocker
                  (usleep 80000)
                  (close-port (car (accept listener)))
                  (close-port blocker))
                (let ((connection (car (accept listener))))
                  (call-with-output-file received
                    (lambda (port) (display (get-string-all connection) port)))
                  (false-if-exception (display response connection))
                  (false-if-exception (force-output connection))
                  (false-if-exception (close-port connection)))
                (primitive-exit 0))
              (lambda _ (primitive-exit 1)))))
        (let* ((started (now-ms))
               (reply (with-syscalls replacements
                        (lambda () (ipc-request path request #:timeout-ms timeout-ms))))
               (elapsed (- (now-ms) started)))
          (when server
            (let ((status (cdr (waitpid server))))
              (set! server #f)
              (test-equal "server finished its one exchange" 0 (status:exit-val status)))
            (test-assert "no replay connection"
              (null? (car (real-select (list listener) '() '() 0)))))
          (list reply elapsed
                (and (file-exists? received) (call-with-input-file received get-string-all)))))
      (lambda ()
        (when server (false-if-exception (kill server SIGKILL)) (waitpid server))
        (when blocker (close-port blocker))
        (close-port listener)
        (when (file-exists? received) (delete-file received))
        (when (file-exists? path) (delete-file path))
        (rmdir directory)))))

(define (check-success result)
  (test-equal "complete reply" '(ok "λ, complete") (car result))
  (test-equal "request delivered exactly once without missing bytes" request (caddr result)))
(define (check-failure result code sent?)
  (let ((reply (car result)))
    (test-equal "transport error" 'error (car reply))
    (test-equal "error code" code (cadr reply))
    (let ((metadata (list-ref reply 5)))
      (test-equal "transport stage" 'transport (assq-ref metadata 'stage))
      (test-equal "possible effects reflect bytes sent" sent?
        (assq-ref metadata 'effects-may-have-occurred?))
      (test-equal "outcome reflects bytes sent" (if sent? 'unknown 'not-started)
        (assq-ref metadata 'outcome)))))

(test-group "partial transfers and transient syscall errors"
  (check-success
    (exchange
      `((connect . ,(interruptions real-connect (list EINTR)))
        (select . ,(interruptions real-select (list EINTR EINTR)))
        (send . ,(interruptions short-send (list EAGAIN #f EINTR EWOULDBLOCK)))
        (recv! . ,(interruptions short-recv! (list EAGAIN #f EINTR EWOULDBLOCK)))))))
(test-group "Guile pending-connect return"
  (check-success
    (exchange `((connect . ,(lambda args (apply real-connect args) #f))))))
(test-group "EINPROGRESS after connecting"
  (check-success
    (exchange `((connect . ,(lambda args
                             (apply real-connect args)
                             (system-failure EINPROGRESS)))))))
(test-group "real full Unix-socket queue drains"
  (check-success (exchange '() #:full-queue? #t)))
(test-group "full queue stays bounded before transmission"
  (let ((result (exchange '() #:full-queue? #t #:drain? #f #:timeout-ms 100)))
    (check-failure result 'timeout #f)
    (test-assert "original deadline" (<= 80 (cadr result) 2000))))
(for-each
  (lambda (entry)
    (test-group (symbol->string (car entry))
      (let ((result (exchange (list (cons (car entry) (always-fails (cadr entry))))
                              #:timeout-ms 100)))
        (check-failure result 'timeout (caddr entry))
        (test-assert "interruptions retain original deadline" (<= 80 (cadr result) 2000))
        (test-equal "only transmitted bytes reach server" (if (caddr entry) request "")
          (caddr result)))))
  `((select ,EINTR #f) (send ,EAGAIN #f) (recv! ,EINTR #t)))
(test-group "hard send failure before any bytes"
  (let ((result (exchange `((send . ,(always-fails EPIPE))))))
    (check-failure result 'connection-failed #f)
    (test-equal "no bytes delivered" "" (caddr result))))
(test-group "hard send failure after partial delivery"
  (let* ((first? #t)
         (result (exchange `((send . ,(lambda (sock bytes)
                                       (if first?
                                           (begin (set! first? #f) (short-send sock bytes))
                                           (system-failure EPIPE))))))))
    (check-failure result 'connection-failed #t)
    (test-equal "only first partial write delivered" "(di" (caddr result))))
(test-group "permanent readiness failure"
  (let ((attempts 0))
    (check-failure
      (exchange `((select . ,(lambda args
                              (set! attempts (+ attempts 1))
                              (system-failure EBADF)))))
      'connection-failed #f)
    (test-equal "readiness errors other than EINTR are not retried" 1 attempts)))
(test-group "permanent errors are not retried"
  (let ((attempts 0))
    (check-failure
      (exchange `((connect . ,(lambda (sock family path)
                               (set! attempts (+ attempts 1))
                               (real-connect sock family (string-append path "-missing")))))
                #:drain? #f)
      'connection-failed #f)
    (test-equal "one failed connection" 1 attempts)))

(let ((runner (test-runner-current)))
  (test-end "control-transport")
  (exit (zero? (+ (test-runner-fail-count runner) (test-runner-xpass-count runner)))))

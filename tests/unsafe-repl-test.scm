;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Exercise real Unix socket permissions without starting an evaluator.

(use-modules (minde unsafe-repl) (ice-9 textual-ports))

(define failures 0)
(define checks 0)
(define (check name value)
  (set! checks (+ checks 1))
  (unless value
    (set! failures (+ failures 1))
    (format #t "FAIL - ~a~%" name)))
(define (rejected? directory)
  (catch #t
    (lambda () (close-port (make-private-repl-socket directory)) #f)
    (lambda _ #t)))

(define root (mkdtemp "/tmp/minde-repl-test-XXXXXX"))
(define runtime (string-append root "/runtime"))
(define path (string-append runtime "/minde-repl.sock"))
(define alias (string-append root "/alias"))

(dynamic-wind
  (lambda () (mkdir runtime #o700))
  (lambda ()
    (for-each (lambda (directory)
                (check "unset, empty and relative directories are rejected"
                       (rejected? directory)))
              (list #f "" "relative"))
    (for-each (lambda (mode)
                (chmod runtime mode)
                (check "group/other-accessible runtime directory is rejected"
                       (rejected? runtime))
                (check "rejection creates no socket" (not (file-exists? path))))
              '(#o755 #o710 #o701 #o740 #o704 #o720 #o702))
    (chmod runtime #o700)
    (symlink runtime alias)
    (check "symlink runtime directory is rejected" (rejected? alias))
    (check "trailing slash cannot hide a symlink"
           (rejected? (string-append alias "/")))
    (delete-file alias)

    (call-with-output-file path (lambda (port) (display "keep me" port)))
    (check "existing regular file is rejected" (rejected? runtime))
    (check "existing regular file is preserved"
           (equal? "keep me" (call-with-input-file path get-string-all)))
    (check "regular file cannot be the runtime directory" (rejected? path))
    (delete-file path)
    (symlink (string-append root "/missing") path)
    (check "dangling socket-path symlink is rejected" (rejected? runtime))
    (check "dangling symlink is preserved" (eq? 'symlink (stat:type (lstat path))))
    (delete-file path)

    (let* ((server (make-private-repl-socket (string-append runtime "/")))
           (inode (stat:ino (lstat path))))
      (check "socket has mode 0600" (= #o600 (stat:perms (lstat path))))
      (check "socket belongs to session user" (= (geteuid) (stat:uid (lstat path))))
      (listen server 1)
      (check "active listener cannot be replaced" (rejected? runtime))
      (check "active socket inode is preserved" (= inode (stat:ino (lstat path))))
      (let ((client (socket PF_UNIX SOCK_STREAM 0)))
        (connect client AF_UNIX path)
        (let ((peer (car (accept server))))
          (display "hello\n" client)
          (force-output client)
          (check "owner can use the original listener" (equal? 'hello (read peer)))
          (close-port peer))
        (close-port client))
      (close-port server)
      (check "stale socket requires explicit cleanup" (rejected? runtime))
      (check "stale socket inode is preserved" (= inode (stat:ino (lstat path))))
      (delete-file path)))
  (lambda ()
    (for-each (lambda (entry)
                (when (false-if-exception (lstat entry)) (delete-file entry)))
              (list path alias))
    (rmdir runtime)
    (rmdir root)))

(format #t "unsafe REPL: ~a checks, ~a failures~%" checks failures)
(exit (if (zero? failures) 0 1))

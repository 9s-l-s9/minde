#!/usr/bin/env -S guix repl -q --
!#
;;; Exercise the privileged, temporary-ACL import on a disposable CI runner.
(use-modules (ice-9 match)
             (ice-9 format)
             (ice-9 textual-ports)
             (guix store)
             (guix nar)
             (guix build utils)
             ((guix build syscalls) #:select (mkdtemp!))
             (gcrypt hash))

(define self (canonicalize-path (current-filename)))
(define contents "Minde signed binary import probe\n")
(define (run! . args)
  (unless (zero? (apply system* args)) (error "command failed" args)))

(unless (equal? (getenv "GITHUB_ACTIONS") "true")
  (error "this test is only for disposable GitHub runners"))

(match (cdr (command-line))
  (("restore" archive expected)
   (unless (zero? (geteuid)) (error "root is required"))
   (when (string=? (readlink "/proc/self/ns/mnt")
                   (readlink (format #f "/proc/~a/ns/mnt" (getppid))))
     (error "missing private mount namespace"))
   (run! "mount" "--bind" "/gnu/store" "/gnu/store")
   (run! "mount" "-o" "remount,bind,rw" "/gnu/store")
   (with-store store
     (add-temp-root store expected)
     (let ((restored (call-with-input-file archive restore-file-set)))
       (unless (and (equal? restored (list expected))
                    (valid-path? store expected)
                    (string=? (call-with-input-file expected get-string-all) contents))
         (error "restored probe is incorrect")))))
  ((guix)
   (let* ((directory (mkdtemp! "/tmp/minde-import-test.XXXXXX"))
          (archive (string-append directory "/probe.nar"))
          (empty (string-append directory "/empty"))
          (trusted (string-append directory "/trusted"))
          (original-acl (file-sha256 "/etc/guix/acl"))
          (probe (with-store store
                   (let ((path (add-text-to-store store "minde-import-probe" contents)))
                     (call-with-output-file archive
                       (lambda (port) (export-paths store (list path) port)))
                     path))))
     (dynamic-wind
       (lambda () #t)
       (lambda ()
         (mkdir empty)
         (mkdir trusted)
         (copy-file "/etc/guix/signing-key.pub"
                    (string-append trusted "/signing-key.pub"))
         (run! guix "gc" "--delete" probe)
         (when (zero? (system* "sudo" "unshare" "--mount" "--propagation" "private"
                              "env" "GITHUB_ACTIONS=true"
                              (string-append "GUIX_CONFIGURATION_DIRECTORY=" empty)
                              guix "repl" "-q" "--" self "restore" archive probe))
           (error "an unauthorized import was accepted"))
         (when (file-exists? probe) (error "failed import registered the probe"))
         (run! "sudo" "unshare" "--mount" "--propagation" "private"
               "env" "GITHUB_ACTIONS=true"
               (string-append "GUIX_CONFIGURATION_DIRECTORY=" trusted)
               guix "repl" "-q" "--" self "restore" archive probe)
         (unless (equal? original-acl (file-sha256 "/etc/guix/acl"))
           (error "the import changed the daemon's global trust"))
         (display "Binary import: rejects untrusted keys, restores signed data, preserves global ACL.\n"))
       (lambda () (run! "sudo" "rm" "-rf" directory)))))
  (_ (error "usage: check-ci-import.scm GUIX")))

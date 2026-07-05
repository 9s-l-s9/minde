;;; SPDX-License-Identifier: GPL-3.0-or-later
(use-modules (ice-9 regex)
             (minde commands)
             (minde command-catalog))

(define failures 0)
(define (check name value)
  (unless value
    (set! failures (+ failures 1))
    (format #t "FAIL - ~a~%" name)))

(register-builtin-command-schemas!)
(define public-modules
  '((minde windows)
    (minde frames)
    (minde groups)
    (minde layouts)
    (minde input)
    (minde commands)
    (minde hooks)
    (minde status)))
(for-each
 (lambda (module-name)
   (check (format #f "public module ~s resolves" module-name)
          (resolve-interface module-name)))
 public-modules)
(check "catalog is non-empty" (pair? (command-names)))
(for-each
 (lambda (name)
   (let ((command (command-ref name)))
     (check (format #f "~a is canonical" name)
            (string-match "^[a-z][a-z0-9]*(-[a-z0-9]+)*[!?]?$"
                          (symbol->string name)))
     (check (format #f "~a has arguments" name) (list? (command-arguments command)))
     (check (format #f "~a has category" name) (symbol? (command-category command)))
     (check (format #f "~a has summary" name)
            (and (string? (command-summary command))
                 (not (string-null? (command-summary command)))))
     (check (format #f "~a has documentation" name)
            (and (string? (command-documentation command))
                 (not (string-null? (command-documentation command)))))
     (check (format #f "~a has demo id" name) (symbol? (command-demo-id command)))))
 (command-names))

(define forbidden-api-names
  '(gnext! gprev! gother! gnew! gnewbg! gkill! gmerge!
    snext! sprev! sother! fclear! only! sibling! banish!
    focus-prev-frame! focus-prev-window!))
(for-each
 (lambda (module-name)
   (let ((interface (resolve-interface module-name)))
     (for-each
      (lambda (name)
        (check (format #f "~s does not export ~a" module-name name)
               (not (module-variable interface name))))
      forbidden-api-names)))
 '((minde frames) (minde groups)))

(if (zero? failures)
    (format #t "all API contract tests passed~%")
    (exit 1))

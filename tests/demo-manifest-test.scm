;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Validate real manifest generation without starting graphical capture.
(use-modules (ice-9 textual-ports) (srfi srfi-1) (srfi srfi-13) (srfi srfi-64))
(test-begin "demo-manifest")
(define root (getcwd))
(define source (call-with-input-file "demos/scenarios.scm" read))
(define manifest (call-with-input-file "doc/generated/demo-manifest.json" get-string-all))
(define (changed name value)
  ;; Change the last scenario to detect any output before validation finishes.
  (append (drop-right source 1)
          (list (cons 'scenario
                      (map (lambda (field) (if (eq? (car field) name) (list name value) field))
                           (cdr (last source)))))))
(let* ((directory (mkdtemp (string-copy "/tmp/minde-demo-manifest-XXXXXX")))
       (demos (string-append directory "/demos"))
       (fixture (string-append demos "/scenarios.scm"))
       (output (string-append directory "/stdout"))
       (errors (string-append directory "/stderr")))
  (define (generate data arguments)
    (call-with-output-file fixture (lambda (port) (write data port)))
    (let ((status
           (call-with-output-file output
             (lambda (out)
               (call-with-output-file errors
                 (lambda (err)
                   (parameterize ((current-output-port out) (current-error-port err))
                     (apply system* "guile" "--no-auto-compile" "-L" (string-append root "/scheme")
                            "-s" (string-append root "/scripts/generate-demo-manifest.scm") arguments))))))))
      (list (status:exit-val status)
            (call-with-input-file output get-string-all)
            (call-with-input-file errors get-string-all))))
  (dynamic-wind
    (lambda () (mkdir demos) (chdir directory))
    (lambda ()
      (let ((result (generate source '())))
        (test-equal "existing JSON manifest succeeds" 0 (car result))
        (test-equal "existing JSON manifest is unchanged" manifest (cadr result)))
      (let* ((result (generate source '("--tsv")))
             (rows (map (lambda (row) (string-split row #\tab))
                        (drop-right (string-split (cadr result) #\newline) 1))))
        (test-equal "existing TSV succeeds" 0 (car result))
        (test-equal "TSV retains every scenario" (length (cddr source)) (length rows))
        (test-assert "every TSV row has seven nonempty columns"
          (every (lambda (row) (and (= (length row) 7) (every (lambda (text) (not (string-null? text))) row))) rows))
        (test-assert "empty input lists retain their sentinel"
          (any (lambda (row) (equal? (list-ref row 3) "-")) rows)))
      (for-each
       (lambda (arguments)
         (for-each
          (lambda (name)
            (for-each
             (lambda (value)
               (let ((result (generate (changed name (if (eq? name 'input) (list value) value)) arguments)))
                 (test-assert (format #f "~a rejects invalid ~a" arguments name) (not (zero? (car result))))
                 (test-equal "invalid scenarios emit no partial manifest" "" (cadr result))
                 (test-assert "invalid field diagnostic"
                   (string-contains (caddr result) (if (eq? name 'input) "demo input" "demo field")))))
             '("" "before\tafter" "before\nafter" "before\rafter")))
          '(title trigger setup action cleanup input)))
       '(() ("--tsv"))))
    (lambda ()
      (chdir root)
      (for-each (lambda (path) (when (file-exists? path) (delete-file path))) (list fixture output errors))
      (rmdir demos) (rmdir directory))))
(let ((runner (test-runner-current)))
  (test-end "demo-manifest")
  (exit (zero? (+ (test-runner-fail-count runner) (test-runner-xpass-count runner)))))

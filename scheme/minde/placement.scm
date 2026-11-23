;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Pure placement matching, file migration and ordering diagnostics.

(define-module (minde placement)
  #:use-module (srfi srfi-1)
  #:export (placement-rules-version make-placement-matcher
            valid-placement-rules? normalize-placement-rules
            migrate-placement-rules placement-rules-from-datum
            placement-rule-matches? placement-rule-warnings))

(define placement-rules-version 2)

(define (nonempty-text? value)
  (and (string? value) (not (string-null? value))))

(define (valid-matcher? matcher)
  (and (list? matcher) (pair? matcher)
       (every (lambda (entry)
                (and (pair? entry)
                     (if (eq? (car entry) 'dialog)
                         (eq? (cdr entry) #t)
                         (and (memq (car entry) '(app-id title legacy))
                              (nonempty-text? (cdr entry))))))
              matcher)
       (= (length matcher) (length (delete-duplicates (map car matcher))))
       (or (not (assq 'legacy matcher)) (= (length matcher) 1))))

(define* (make-placement-matcher legacy app-id title #:optional (dialog? #f))
  (when (and legacy (or app-id title dialog?))
    (error "use either the deprecated string matcher or #:app-id / #:title / #:dialog?"))
  (let ((matcher (if legacy `((legacy . ,legacy))
                     (append (if app-id `((app-id . ,app-id)) '())
                             (if title `((title . ,title)) '())
                             (if dialog? '((dialog . #t)) '())))))
    (unless (valid-matcher? matcher)
      (error "placement requires a nonempty #:app-id or #:title, or #:dialog? #t"))
    matcher))

(define (valid-rule-fields? rule)
  (and (or (not (cadr rule)) (string? (cadr rule)))
       (exact-integer? (caddr rule)) (>= (caddr rule) 0)
       (every boolean? (cdddr rule))))

(define (valid-placement-rules? rules)
  (and (list? rules)
       (every (lambda (rule)
                (and (list? rule) (= (length rule) 6)
                     (valid-matcher? (car rule)) (valid-rule-fields? rule)))
              rules)))

(define (normalize-placement-rules rules)
  ;; Canonical matcher order makes equivalent hand-written rules deduplicate.
  (delete-duplicates
   (map (lambda (rule)
          (cons (filter-map (lambda (key) (assq key (car rule)))
                            '(app-id title dialog legacy))
                (cdr rule)))
        rules)))

(define (migrate-placement-rules version rules)
  (unless (and (= version 1) (list? rules)
               (every (lambda (rule)
                        (and (list? rule) (<= 4 (length rule) 6)
                             (nonempty-text? (car rule))
                             (valid-rule-fields? rule)))
                      rules))
    (error "unsupported or invalid placement rules" version rules))
  (map (lambda (rule)
         (append (list `((legacy . ,(car rule)))) (take (cdr rule) 3)
                 (list (or (< (length rule) 5) (list-ref rule 4))
                       (and (= (length rule) 6) (list-ref rule 5)))))
       rules))

(define (placement-rules-from-datum datum)
  "Validates a versioned rules datum and returns version-2 rules.
Keeps duplicates so diagnostics refer to the original file's rule positions."
  (unless (and (list? datum) (= (length datum) 3)
               (eq? (car datum) 'minde-placement-rules)
               (exact-integer? (cadr datum)))
    (error "invalid placement rules file" datum))
  (let ((rules (if (= (cadr datum) placement-rules-version) (caddr datum)
                   (migrate-placement-rules (cadr datum) (caddr datum)))))
    (unless (valid-placement-rules? rules) (error "invalid placement rules" rules))
    rules))

(define* (placement-rule-matches? rule title app-id #:optional (dialog? #f))
  "True when RULE matches. DIALOG? says whether the window is a dialog
(transient or dialog-type); only rules with a dialog selector look at it."
  (let* ((matcher (car rule))
         (legacy (assq-ref matcher 'legacy))
         (app (assq-ref matcher 'app-id))
         (text (assq-ref matcher 'title)))
    (and (or (not (assq 'dialog matcher)) dialog?)
         (if legacy
             (if (nonempty-text? app-id) (string=? app-id legacy)
                 (and (string? title) (string-contains title legacy)))
             (and (or (not app) (and (string? app-id) (string=? app-id app)))
                  (or (not text) (and (string? title) (string-contains title text)))))
         #t)))

(define (matcher-covers? broad narrow)
  (let ((old (assq-ref broad 'legacy)) (new (assq-ref narrow 'legacy))
        (app (assq-ref broad 'app-id)) (other-app (assq-ref narrow 'app-id))
        (title (assq-ref broad 'title)) (other-title (assq-ref narrow 'title))
        (dialog (assq-ref broad 'dialog)) (other-dialog (assq-ref narrow 'dialog)))
    (cond
     ;; Legacy rules also match titles on clients without an app-id.
     (old (or (equal? old new) (equal? old other-app)))
     (new #f)
     (else (and (or (not dialog) other-dialog)
                (or (not app) (equal? app other-app))
                (or (not title)
                    (and other-title (string-contains other-title title))))))))

(define (placement-rule-warnings rules)
  "Returns diagnostics for rules shadowed during initial placement.
Unlocked earlier rules do not shadow locked rules at map time."
  (let loop ((remaining rules) (earlier '()) (index 1) (warnings '()))
    (if (null? remaining) (reverse warnings)
        (let* ((rule (car remaining))
               (shadow (find (lambda (entry)
                               (let ((prior (cdr entry)))
                                 (and (or (list-ref prior 4) (not (list-ref rule 4)))
                                      (matcher-covers? (car prior) (car rule)))))
                             earlier)))
          (loop (cdr remaining) (append earlier (list (cons index rule)))
                (+ index 1)
                (if shadow
                    (cons (format #f "placement rule ~a is shadowed for initial placement by earlier rule ~a"
                                  index (car shadow)) warnings)
                    warnings))))))

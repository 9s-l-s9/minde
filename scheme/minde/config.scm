;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Declarative configuration parsing and validation. No compositor effects.

(define-module (minde config)
  #:use-module (srfi srfi-1)
  #:use-module (minde foundation keys)
  #:use-module (minde foundation serialization)
  #:use-module (minde commands)
  #:export (read-configuration
            validate-configuration
            validate-configuration-file
            configuration-prefix-modifiers
            configuration-prefix-key
            configuration-bindings))

(define (field name datum)
  (let ((matches (filter (lambda (entry) (and (pair? entry) (eq? name (car entry))))
                         (cdr datum))))
    (unless (= (length matches) 1) (error "configuration field must occur once" name))
    (car matches)))

(define (validate-binding binding seen)
  (unless (and (list? binding) (= (length binding) 2)
               (string? (car binding)) (symbol? (cadr binding)))
    (error "binding must be (key command-name)" binding))
  (when (string-null? (car binding)) (error "binding key cannot be empty" binding))
  (unless (command-ref (cadr binding)) (error "binding names unknown command" binding))
  (when (member (car binding) seen) (error "duplicate binding key" (car binding)))
  (car binding))

(define (validate-configuration datum)
  "Validate and return DATUM without applying compositor state."
  (unless (and (list? datum) (eq? (car datum) 'minde-config))
    (error "configuration must start with minde-config"))
  (let* ((version-field (field 'version datum))
         (prefix-field (field 'prefix datum))
         (bindings-field (field 'bindings datum))
         (allowed '(version prefix bindings)))
    (for-each (lambda (entry)
                (unless (and (pair? entry) (memq (car entry) allowed))
                  (error "unknown configuration field" entry)))
              (cdr datum))
    (unless (equal? version-field '(version 1))
      (error "unsupported configuration version" version-field))
    (unless (and (= (length prefix-field) 3)
                 (list? (cadr prefix-field))
                 (every symbol? (cadr prefix-field))
                 (string? (caddr prefix-field)))
      (error "prefix must be (prefix (modifiers ...) key)" prefix-field))
    ;; Force modifier validation now rather than when the candidate is applied.
    (modifiers->bitmask (cadr prefix-field))
    (let loop ((bindings (cdr bindings-field)) (seen '()))
      (unless (null? bindings)
        (loop (cdr bindings) (cons (validate-binding (car bindings) seen) seen))))
    datum))

(define (read-configuration path) (read-datum-file path))
(define (validate-configuration-file path)
  (validate-configuration (read-configuration path)))
(define (configuration-prefix-modifiers config) (cadr (field 'prefix config)))
(define (configuration-prefix-key config) (caddr (field 'prefix config)))
(define (configuration-bindings config) (cdr (field 'bindings config)))

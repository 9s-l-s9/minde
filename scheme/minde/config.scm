;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Declarative configuration parsing and validation. No compositor effects.

(define-module (minde config)
  #:use-module (srfi srfi-1)
  #:use-module (minde foundation keys)
  #:use-module (minde foundation serialization)
  #:use-module (minde commands)
  #:use-module (minde keysyms)
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

(define (canonical-binding-key notation)
  (let loop ((key notation) (modifiers '()))
    (let ((separator (string-index key #\-)))
      (if separator
          (let ((modifier (assoc (substring key 0 separator)
                                 '(("C" . ctrl) ("M" . alt)
                                   ("S" . shift) ("s" . super)))))
            (unless modifier (error "unknown key modifier" notation))
            (loop (substring key (+ separator 1)) (cons (cdr modifier) modifiers)))
          (key-notation modifiers (canonical-keysym-name key))))))

(define (validate-binding binding seen)
  (unless (and (list? binding) (= (length binding) 2)
               (string? (car binding)) (symbol? (cadr binding)))
    (error "binding must be (key command-name)" binding))
  (let ((command (command-ref (cadr binding))))
    (unless command (error "binding names unknown command" binding))
    (unless (null? (command-arguments command))
      (error "key binding command requires arguments" (command-name command))))
  (let ((key (canonical-binding-key (car binding))))
    (when (member key seen) (error "duplicate binding key" key))
    (list key (cadr binding))))

(define (validate-configuration datum)
  "Validate DATUM and return canonical keys without applying compositor state."
  (unless (and (list? datum) (pair? datum) (eq? (car datum) 'minde-config))
    (error "configuration must start with minde-config"))
  (let* ((version-field (field 'version datum))
         (prefix-field (field 'prefix datum))
         (bindings-field (field 'bindings datum))
         (allowed '(version prefix bindings)))
    (for-each (lambda (entry)
                (unless (and (list? entry) (pair? entry) (memq (car entry) allowed))
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
    (let ((prefix-key (canonical-keysym-name (caddr prefix-field)))
          (bindings
           (let loop ((remaining (cdr bindings-field)) (seen '()) (result '()))
             (if (null? remaining) (reverse result)
                 (let ((binding (validate-binding (car remaining) seen)))
                   (loop (cdr remaining) (cons (car binding) seen) (cons binding result)))))))
      ;; Preserve field order and leave the caller's data untouched.
      (cons 'minde-config
            (map (lambda (entry)
                   (case (car entry)
                     ((prefix) (list 'prefix (cadr prefix-field) prefix-key))
                     ((bindings) (cons 'bindings bindings))
                     (else entry)))
                 (cdr datum))))))

(define (read-configuration path)
  (with-fluids ((read-eval? #f)) (read-datum-file path)))
(define (validate-configuration-file path)
  (validate-configuration (read-configuration path)))
(define (configuration-prefix-modifiers config) (cadr (field 'prefix config)))
(define (configuration-prefix-key config) (caddr (field 'prefix config)))
(define (configuration-bindings config) (cdr (field 'bindings config)))

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
            configuration-emergency-exit
            configuration-global-bindings
            configuration-bindings))

(define* (field name datum #:optional default)
  (let ((matches (filter (lambda (entry) (and (pair? entry) (eq? name (car entry))))
                         (cdr datum))))
    (cond ((and (null? matches) default) default)
          ((= (length matches) 1) (car matches))
          (else (error "configuration field must occur once" name)))))

(define %default-emergency-exit '(emergency-exit (ctrl alt) "BackSpace"))

(define (validate-emergency-exit entry)
  (if (equal? entry '(emergency-exit #f)) entry
      (begin
        (unless (and (= (length entry) 3) (list? (cadr entry))
                     (every symbol? (cadr entry)) (string? (caddr entry)))
          (error "emergency-exit must be (emergency-exit (modifiers ...) key) or (emergency-exit #f)" entry))
        (modifiers->bitmask (cadr entry))
        (list 'emergency-exit (cadr entry) (canonical-keysym-name (caddr entry))))))

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

(define (validate-action action)
  (cond
   ((symbol? action)
    (let ((command (command-ref action)))
      (unless command (error "binding names unknown command" action))
      (unless (null? (command-arguments command))
        (error "key binding command requires arguments" action)))
    action)
   ((and (list? action) (= (length action) 3) (eq? (car action) 'call)
         (symbol? (cadr action)))
    (let ((command (command-ref (cadr action))))
      (unless (and command (command-typed? command))
        (error "binding call requires a command with typed parameters" action))
      (validate-command-arguments (cadr action) (caddr action)))
    action)
   (else (error "action must be a command name or (call command arguments)" action))))

(define (validate-keymap action)
  (for-each
   (lambda (entry)
     (unless (and (list? entry) (pair? entry)
                  (memq (car entry) '(description enter bindings)))
       (error "unknown keymap field" entry)))
   (cdr action))
  (let ((description (field 'description action))
        (enter (field 'enter action '(enter #f)))
        (bindings (field 'bindings action)))
    (unless (and (= (length description) 2) (string? (cadr description))
                 (not (string-null? (cadr description))))
      (error "keymap description must be nonempty text" description))
    (unless (= (length enter) 2) (error "invalid keymap enter action" enter))
    (when (cadr enter) (validate-action (cadr enter)))
    (list 'keymap description enter (cons 'bindings (validate-bindings (cdr bindings))))))

(define (validate-binding binding seen)
  (unless (and (list? binding) (memv (length binding) '(2 3))
               (string? (car binding))
               (or (= (length binding) 2)
                   (and (string? (caddr binding)) (not (string-null? (caddr binding))))))
    (error "binding must be (key action [description])" binding))
  (let ((key (canonical-binding-key (car binding))))
    (when (member key seen) (error "duplicate binding key" key))
    (cons key
          (cons (let ((action (cadr binding)))
                  (cond ((eq? action #f) #f)
                        ((and (list? action) (pair? action) (eq? (car action) 'keymap))
                         (validate-keymap action))
                        (else (validate-action action))))
                (cddr binding)))))

(define (validate-bindings bindings)
  (let loop ((remaining bindings) (seen '()) (result '()))
    (if (null? remaining) (reverse result)
        (let ((binding (validate-binding (car remaining) seen)))
          (loop (cdr remaining) (cons (car binding) seen) (cons binding result))))))

(define (validate-configuration datum)
  "Validate DATUM and return canonical keys without applying compositor state."
  (unless (and (list? datum) (pair? datum) (eq? (car datum) 'minde-config))
    (error "configuration must start with minde-config"))
  (let* ((version-field (field 'version datum))
         (prefix-field (field 'prefix datum))
         (bindings-field (field 'bindings datum))
         (emergency-field (field 'emergency-exit datum %default-emergency-exit))
         (global-field (field 'global-bindings datum '(global-bindings)))
         (allowed '(version prefix bindings global-bindings emergency-exit)))
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
          (emergency-exit (validate-emergency-exit emergency-field))
          (globals (validate-bindings (cdr global-field)))
          (bindings (validate-bindings (cdr bindings-field))))
      ;; Preserve field order and leave the caller's data untouched.
      (cons 'minde-config
            (map (lambda (entry)
                   (case (car entry)
                     ((prefix) (list 'prefix (cadr prefix-field) prefix-key))
                     ((bindings) (cons 'bindings bindings))
                     ((global-bindings) (cons 'global-bindings globals))
                     ((emergency-exit) emergency-exit)
                     (else entry)))
                 (cdr datum))))))

(define (read-configuration path)
  (with-fluids ((read-eval? #f)) (read-datum-file path)))
(define (validate-configuration-file path)
  (validate-configuration (read-configuration path)))
(define (configuration-prefix-modifiers config) (cadr (field 'prefix config)))
(define (configuration-prefix-key config) (caddr (field 'prefix config)))
(define (configuration-emergency-exit config)
  (let ((entry (field 'emergency-exit config %default-emergency-exit)))
    (and (cadr entry) (cdr entry))))
(define (configuration-bindings config) (cdr (field 'bindings config)))
(define (configuration-global-bindings config)
  (cdr (field 'global-bindings config '(global-bindings))))

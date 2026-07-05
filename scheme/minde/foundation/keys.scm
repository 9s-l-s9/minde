;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Key modifier notation and collision-detecting registries.

(define-module (minde foundation keys)
  #:export (modifier->bit modifiers->bitmask key-notation
            make-key-registry register-key! lookup-key registered-keys))

(define (modifier->bit modifier)
  (case modifier
    ((shift) 1) ((ctrl control) 4) ((alt meta) 8) ((super logo) 64)
    (else (error "unknown modifier" modifier))))

(define (modifiers->bitmask modifiers)
  (apply + (map modifier->bit modifiers)))

(define (key-notation modifiers key)
  (let ((mask (if (number? modifiers) modifiers (modifiers->bitmask modifiers))))
    (string-append (if (logtest mask 4) "C-" "")
                   (if (logtest mask 8) "M-" "")
                   (if (logtest mask 1) "S-" "")
                   (if (logtest mask 64) "s-" "") key)))

(define (make-key-registry) (make-hash-table))

(define (register-key! registry modifiers key value)
  (let ((notation (key-notation modifiers key)))
    (when (hash-ref registry notation)
      (error "duplicate key binding" notation))
    (hash-set! registry notation value)))

(define (lookup-key registry modifiers key)
  (hash-ref registry (key-notation modifiers key)))

(define (registered-keys registry)
  (hash-map->list (lambda (key value) (cons key value)) registry))

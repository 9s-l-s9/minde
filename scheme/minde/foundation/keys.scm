;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Key modifier notation and collision-detecting registries.

(define-module (minde foundation keys)
  #:export (modifier->bit modifiers->bitmask key-notation
            make-key-registry register-key! lookup-key registered-keys))

(define (modifier->bit modifier)
  "Returns the compositor bit assigned to MODIFIER; errors on unknown names."
  (case modifier
    ((shift) 1) ((ctrl control) 4) ((alt meta) 8) ((super logo) 64)
    (else (error "unknown modifier" modifier))))

(define (modifiers->bitmask modifiers)
  "Returns the sum of the canonical bits for MODIFIERS."
  (apply + (map modifier->bit modifiers)))

(define (key-notation modifiers key)
  "Returns canonical C-/M-/S-/s- notation for MODIFIERS and KEY."
  (let ((mask (if (number? modifiers) modifiers (modifiers->bitmask modifiers))))
    (string-append (if (logtest mask 4) "C-" "")
                   (if (logtest mask 8) "M-" "")
                   (if (logtest mask 1) "S-" "")
                   (if (logtest mask 64) "s-" "") key)))

(define (make-key-registry)
  "Returns an empty collision-detecting key registry."
  (make-hash-table))

(define (register-key! registry modifiers key value)
  "Registers VALUE under MODIFIERS and KEY; errors on a duplicate notation."
  (let ((notation (key-notation modifiers key)))
    (when (hash-ref registry notation)
      (error "duplicate key binding" notation))
    (hash-set! registry notation value)))

(define (lookup-key registry modifiers key)
  "Returns the value registered for MODIFIERS and KEY, or #f."
  (hash-ref registry (key-notation modifiers key)))

(define (registered-keys registry)
  "Returns REGISTRY entries as canonical-notation/value pairs."
  (hash-map->list (lambda (key value) (cons key value)) registry))

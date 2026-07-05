;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Generic immutable binary split trees.

(define-module (minde foundation tree)
  #:use-module (srfi srfi-9)
  #:use-module (ice-9 match)
  #:export (make-leaf leaf? leaf-value
            make-split split? split-orientation split-ratio
            split-first split-second
            tree-leaves tree-map tree-fold tree-valid?
            tree->sexp sexp->tree))

(define-record-type <leaf>
  (make-leaf value) leaf? (value leaf-value))

(define-record-type <split>
  (make-split orientation ratio first second)
  split?
  (orientation split-orientation)
  (ratio split-ratio)
  (first split-first)
  (second split-second))

(define (tree-leaves tree)
  (if (leaf? tree)
      (list tree)
      (append (tree-leaves (split-first tree))
              (tree-leaves (split-second tree)))))

(define (tree-map procedure tree)
  (if (leaf? tree)
      (make-leaf (procedure (leaf-value tree)))
      (make-split (split-orientation tree) (split-ratio tree)
                  (tree-map procedure (split-first tree))
                  (tree-map procedure (split-second tree)))))

(define (tree-fold leaf-procedure split-procedure tree)
  (if (leaf? tree)
      (leaf-procedure (leaf-value tree))
      (split-procedure
       (split-orientation tree) (split-ratio tree)
       (tree-fold leaf-procedure split-procedure (split-first tree))
       (tree-fold leaf-procedure split-procedure (split-second tree)))))

(define (tree-valid? tree)
  (or (leaf? tree)
      (and (split? tree)
           (memq (split-orientation tree) '(horizontal vertical))
           (number? (split-ratio tree)) (< 0 (split-ratio tree) 1)
           (tree-valid? (split-first tree))
           (tree-valid? (split-second tree)))))

(define (tree->sexp tree)
  (tree-fold (lambda (value) (list 'leaf value))
             (lambda (orientation ratio first second)
               (list 'split orientation ratio first second))
             tree))

(define (sexp->tree sexp)
  (match sexp
    (('leaf value) (make-leaf value))
    (('split orientation ratio first second)
     (let ((tree (make-split orientation ratio
                             (sexp->tree first) (sexp->tree second))))
       (if (tree-valid? tree) tree (error "invalid split tree" sexp))))
    (_ (error "invalid split tree" sexp))))

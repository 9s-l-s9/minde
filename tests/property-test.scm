;;; SPDX-License-Identifier: GPL-3.0-or-later
(add-to-load-path "tests")
(use-modules (srfi srfi-1)
             (minde test-fixtures)
             (minde foundation geometry)
             (minde foundation serialization)
             (minde foundation tree))

(define (generated-tree depth first-value seed)
  (if (zero? depth)
      (make-leaf first-value)
      (let* ((ratio (/ (+ 1 (modulo seed 9)) 10))
             (orientation (if (even? seed) 'horizontal 'vertical))
             (left (generated-tree (- depth 1) (* 2 first-value)
                                   (+ seed 17)))
             (right (generated-tree (- depth 1) (+ 1 (* 2 first-value))
                                    (+ seed 31))))
        (make-split orientation ratio left right))))

(for-each
 (lambda (seed)
   (let* ((depth (modulo seed 6))
          (tree (generated-tree depth 1 seed))
          (values (map leaf-value (tree-leaves tree)))
          (round-trip (sexp->tree (tree->sexp tree)))
          (mapped (tree-map (lambda (value) (+ value 1000)) tree)))
     (check (format #f "tree invariant seed ~a" seed) (tree-valid? tree))
     (check-equal (format #f "tree round trip seed ~a" seed)
                  (tree->sexp round-trip) (tree->sexp tree))
     (check-equal (format #f "tree-map preserves leaf order seed ~a" seed)
                  (map leaf-value (tree-leaves mapped))
                  (map (lambda (value) (+ value 1000)) values))
     (check-equal (format #f "tree-fold counts leaves seed ~a" seed)
                  (tree-fold (lambda (_) 1)
                             (lambda (_orientation _ratio first second)
                               (+ first second))
                             tree)
                  (expt 2 depth))))
 (deterministic-integers 40))

;; Directional focus on regular output/frame grids must select the immediately
;; adjacent cell.  Gapped grids exercise cross-output bezel selection.
(for-each
 (lambda (seed)
   (let* ((columns (+ 3 (modulo seed 3)))
          (rows (+ 3 (modulo (quotient seed 7) 3)))
          (width (+ 20 (modulo seed 200)))
          (height (+ 20 (modulo (quotient seed 11) 160)))
          (gap (if (even? seed) 0 (+ 1 (modulo seed 20))))
          (rectangles
           (append-map
            (lambda (row)
              (map (lambda (column)
                     (list (* column (+ width gap))
                           (* row (+ height gap)) width height))
                   (iota columns)))
            (iota rows)))
          (at (lambda (column row)
                (list-ref rectangles (+ column (* row columns)))))
          (source (at 1 1)))
     (for-each
      (lambda (entry)
        (check-equal
         (format #f "direction ~a seed ~a" (car entry) seed)
         (directional-neighbor source rectangles (car entry)
                               #:adjacent? (zero? gap))
         (apply at (cdr entry))))
      '((left 0 1) (right 2 1) (up 1 0) (down 1 2)))))
 (deterministic-integers 30 90210))

;; Output unions include non-uniform rectangles and negative origins.
(for-each
 (lambda (seed)
   (let* ((rectangles
           (map (lambda (index)
                  (let ((value (+ seed (* index 7919))))
                    (list (- (modulo value 4000) 2000)
                          (- (modulo (quotient value 3) 2400) 1200)
                          (+ 1 (modulo (quotient value 5) 1600))
                          (+ 1 (modulo (quotient value 7) 1200)))))
                (iota (+ 1 (modulo seed 8)))))
          (union (rect-union rectangles)))
     (check (format #f "output union contains rectangles seed ~a" seed)
            (every (lambda (rect)
                     (and (<= (car union) (car rect))
                          (<= (cadr union) (cadr rect))
                          (>= (rect-right union) (rect-right rect))
                          (>= (rect-bottom union) (rect-bottom rect))))
                   rectangles))
     (check-equal (format #f "output union left edge seed ~a" seed)
                  (car union) (apply min (map car rectangles)))
     (check-equal (format #f "output union bottom edge seed ~a" seed)
                  (rect-bottom union) (apply max (map rect-bottom rectangles)))))
 (deterministic-integers 30 42))

(for-each
 (lambda (seed)
   (let ((datum `(state ,seed
                        (symbol . ,(string->symbol (format #f "s~a" seed)))
                        #(,seed ,(- seed) #t #f)
                        (text ,(format #f "line ~a\nquote \" slash \\" seed)))))
     (check-equal (format #f "datum round trip seed ~a" seed)
                  (string->datum (datum->string datum)) datum)))
 (deterministic-integers 30 616))
(check-error "serialization rejects trailing datum"
             (lambda () (string->datum "(valid) (unexpected)")))
(check-error "tree decoder rejects invalid ratio"
             (lambda () (sexp->tree '(split horizontal 2 (leaf a) (leaf b)))))

(finish-tests "property")

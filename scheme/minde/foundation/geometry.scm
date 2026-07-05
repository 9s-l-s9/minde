;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Pure rectangle and directional-neighbor operations.

(define-module (minde foundation geometry)
  #:use-module (srfi srfi-1)
  #:export (rect?
            rect-right
            rect-bottom
            rect-overlap
            rect-union
            directional-neighbor))

(define (rect? value)
  (and (list? value) (= (length value) 4) (every number? value)
       (>= (caddr value) 0) (>= (cadddr value) 0)))

(define (rect-right rect) (+ (car rect) (caddr rect)))
(define (rect-bottom rect) (+ (cadr rect) (cadddr rect)))

(define (rect-overlap a1 a2 b1 b2)
  (max 0 (- (min a2 b2) (max a1 b1))))

(define (rect-union rects)
  (if (null? rects)
      #f
      (let ((x1 (apply min (map car rects)))
            (y1 (apply min (map cadr rects)))
            (x2 (apply max (map rect-right rects)))
            (y2 (apply max (map rect-bottom rects))))
        (list x1 y1 (- x2 x1) (- y2 y1)))))

(define* (directional-neighbor origin candidates direction
                               #:key (rectangle identity)
                               (same? eq?) (adjacent? #t))
  "Return the candidate in DIRECTION with the greatest perpendicular
overlap. RECTANGLE maps each object to (x y width height). When ADJACENT?
is false, candidates may be separated by a gap."
  (let* ((source (rectangle origin))
         (x (car source)) (y (cadr source))
         (right (rect-right source)) (bottom (rect-bottom source)))
    (let loop ((rest candidates) (best #f) (best-overlap 0) (best-gap #f))
      (if (null? rest)
          best
          (let* ((candidate (car rest))
                 (rect (rectangle candidate))
                 (cx (car rect)) (cy (cadr rect))
                 (cright (rect-right rect)) (cbottom (rect-bottom rect))
                 (gap (case direction
                        ((left) (- x cright)) ((right) (- cx right))
                        ((up) (- y cbottom)) ((down) (- cy bottom))
                        (else (error "unknown direction" direction))))
                 (in-direction? (and (not (same? candidate origin))
                                     (if adjacent? (= gap 0) (>= gap 0))))
                 (overlap (and in-direction?
                               (if (memq direction '(left right))
                                   (rect-overlap y bottom cy cbottom)
                                   (rect-overlap x right cx cright))))
                 (better? (and overlap (> overlap 0)
                               (or (not best)
                                   (< gap best-gap)
                                   (and (= gap best-gap)
                                        (> overlap best-overlap))))))
            (if better?
                (loop (cdr rest) candidate overlap gap)
                (loop (cdr rest) best best-overlap best-gap)))))))

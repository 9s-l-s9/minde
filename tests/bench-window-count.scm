;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Compare duplicate-aware window counts; compile with guild before timing.
(use-modules (srfi srfi-1) (ice-9 format))
(define (old ids) (length (delete-duplicates ids)))
(use-modules (minde groups))
(define new (@@ (minde groups) distinct-window-count))
(for-each
 (lambda (n)
   (let ((ids (append (iota n) (iota (quotient n 4)))))
     (unless (= (old ids) (new ids)) (error "mismatch"))
     (for-each
      (lambda (f)
        (let ((start (get-internal-real-time)) (iterations (max 20 (quotient 20000 n))))
          (do ((i 0 (+ i 1))) ((= i iterations)) (f ids))
          (format #t "~a ~a ~,2f us~%" n (if (eq? f old) 'list 'hash)
                  (* 1e6 (/ (- (get-internal-real-time) start)
                             internal-time-units-per-second iterations)))))
      (list old new))))
 '(12 100 1000))

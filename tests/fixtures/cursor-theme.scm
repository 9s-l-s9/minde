;; SPDX-License-Identifier: GPL-3.0-or-later
;; Generate an opaque Xcursor fixture with asymmetric, nonzero hotspots.
;; Keep binary images in the test output directory, not in the repository.
(use-modules (ice-9 binary-ports) (rnrs bytevectors))

(let ((port (open-file (cadr (command-line)) "wb"))
      (sizes '(24 48 72)))
  (define (word value)
    (let ((bytes (make-bytevector 4)))
      (bytevector-u32-set! bytes 0 value (endianness little))
      (put-bytevector port bytes)))
  (put-bytevector port #vu8(88 99 117 114)) ; Xcur
  (for-each word (list 16 #x10000 (length sizes)))
  (let loop ((rest sizes) (offset (+ 16 (* 12 (length sizes)))))
    (unless (null? rest)
      (let ((size (car rest)))
        (for-each word (list #xfffd0002 size offset))
        (loop (cdr rest) (+ offset 36 (* size size 4))))))
  (for-each
   (lambda (size)
     (for-each word (list 36 #xfffd0002 size 1 size size
                          (+ 1 (quotient size 2)) (+ 1 (quotient size 3)) 0))
     (do ((pixel 0 (+ pixel 1))) ((= pixel (* size size)))
       (word #xff12e0a7)))
   sizes)
  (close-port port))

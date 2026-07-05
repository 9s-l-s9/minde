;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Safe single-datum serialization helpers.

(define-module (minde foundation serialization)
  #:export (datum->string string->datum
            write-datum-file read-datum-file
            write-versioned-datum-file read-versioned-datum-file))

(define (datum->string datum)
  (call-with-output-string (lambda (port) (write datum port))))

(define (string->datum text)
  (call-with-input-string text
    (lambda (port)
      (let ((datum (read port)))
        (unless (eof-object? (read port))
          (error "expected exactly one datum"))
        datum))))

(define (write-datum-file path datum)
  "Atomically replaces PATH with DATUM. The temporary file is created in
PATH's directory so rename is atomic on the target filesystem."
  (let ((temporary (string-append path ".tmp." (number->string (getpid)))))
    (catch #t
      (lambda ()
        (call-with-output-file temporary
          (lambda (port)
            (write datum port)
            (newline port)
            (force-output port)))
        (rename-file temporary path))
      (lambda arguments
        (when (file-exists? temporary) (delete-file temporary))
        (apply throw arguments)))))

(define (read-datum-file path)
  (call-with-input-file path
    (lambda (port)
      (let ((datum (read port)))
        (unless (eof-object? (read port))
          (error "expected exactly one datum" path))
        datum))))

(define (write-versioned-datum-file path kind version payload)
  (write-datum-file path (list kind version payload)))

(define (read-versioned-datum-file path kind version)
  (let ((datum (read-datum-file path)))
    (unless (and (list? datum)
                 (= (length datum) 3)
                 (eq? (car datum) kind)
                 (integer? (cadr datum))
                 (= (cadr datum) version))
      (error "unsupported persistent state format" path kind version datum))
    (caddr datum)))

;;; ipc-reply.scm -- bounded, readable IPC reply envelopes.
;;;
;;; Loaded by init.scm and headless tests. The legacy positional fields stay:
;;;   (ok RESULT)
;;;   (error KEY ARGS MESSAGE-STRING BACKTRACE-STRING METADATA)
;;; METADATA is an alist describing the failure stage and possible effects.
;;; Raw eval is never transactional: a result-encoding failure means evaluation
;;; completed, and an execution failure may follow successful mutations.
;;;
;;; Supported data: booleans, numbers, characters, symbols, keywords, strings,
;;; null, pairs (including dotted pairs), vectors, and bytevectors. Shared
;;; acyclic data is copied; cyclic and opaque values are rejected. Guile's
;;; unspecified value becomes the explicit datum (unspecified). No user object
;;; printer is called to classify a value.

(use-modules (ice-9 hash-table) (rnrs bytevectors) (system vm frame))

(define %ipc-backtrace-max-frames 8)
(define %ipc-backtrace-max-chars 2000)
(define %ipc-message-max-chars 2000)
(define %ipc-reply-max-chars 262144)
(define %ipc-data-max-nodes 16384)
(define %ipc-data-max-depth 128)

(define (ipc-truncate string limit)
  "Returns STRING bounded to LIMIT characters, including the marker."
  (let ((marker "...[truncated]"))
    (if (> (string-length string) limit)
        (string-append (substring string 0 (max 0 (- limit (string-length marker))))
                       (substring marker 0 (min limit (string-length marker))))
        string)))

(define (ipc-capture-output writer limit truncate?)
  "Calls WRITER with a bounded textual port. Stop at LIMIT characters before
allocating unbounded printer output. Truncation is only for diagnostic strings;
wire datums fail in full rather than becoming malformed Scheme."
  (let ((output (open-output-string)) (remaining limit) (overflow? #f))
    (catch 'ipc-output-limit
      (lambda ()
        (let* ((emit (lambda (text)
                       (let ((n (string-length text)))
                         (if (> n remaining)
                             (begin
                               (display (substring text 0 remaining) output)
                               (set! remaining 0)
                               (set! overflow? #t)
                               (throw 'ipc-output-limit))
                             (begin (display text output)
                                    (set! remaining (- remaining n)))))))
               (port (make-soft-port
                      (vector (lambda (char) (emit (string char))) emit
                              (lambda () #t) #f (lambda () #t))
                      "w")))
          (writer port)
          (force-output port)))
      (lambda _ #f))
    (if overflow?
        (if truncate?
            ;; ipc-truncate installs the marker inside the configured bound.
            (ipc-truncate (string-append (get-output-string output) " ") limit)
            (throw 'ipc-data-error 'result-too-large))
        (get-output-string output))))

(define* (ipc-normalize-datum value #:optional (sanitize? #f))
  "Copies supported VALUE into bounded wire data. SANITIZE? substitutes short
strings for opaque/cyclic values, for error irritants and legacy event payloads.
Node, depth, and aggregate content budgets bound traversal before printing."
  (let ((active (make-hash-table)) (nodes 0) (content 0))
    (define (charge! amount)
      (set! content (+ content amount))
      (when (> content %ipc-reply-max-chars)
        (throw 'ipc-data-error 'result-too-large)))
    (define (unsupported code)
      (if sanitize?
          (case code
            ((cyclic-result) "<cyclic-value>")
            (else "<unsupported-value>"))
          (throw 'ipc-data-error code)))
    (define (number-size! number)
      ;; Avoid converting an enormous bignum to decimal before limiting it.
      ;; Bit length is a deliberately conservative upper bound on its digits.
      (cond
       ((not (exact? number)) (charge! 64))
       ((integer? number) (charge! (+ 2 (integer-length number))))
       ((real? number)
        (number-size! (numerator number))
        (number-size! (denominator number)))
       (else (number-size! (real-part number))
             (number-size! (imag-part number)))))
    (define (walk value depth)
      (set! nodes (+ nodes 1))
      (when (> nodes %ipc-data-max-nodes)
        (throw 'ipc-data-error 'result-too-large))
      (when (> depth %ipc-data-max-depth)
        (throw 'ipc-data-error 'result-too-deep))
      (charge! 1)
      (cond
       ((unspecified? value) '(unspecified))
       ((or (null? value) (boolean? value) (char? value)) value)
       ((number? value) (number-size! value) value)
       ((string? value) (charge! (string-length value)) value)
       ((symbol? value) (charge! (string-length (symbol->string value))) value)
       ((keyword? value)
        (charge! (string-length (symbol->string (keyword->symbol value)))) value)
       ((bytevector? value)
        (charge! (* 4 (bytevector-length value))) value)
       ((or (pair? value) (vector? value))
        (if (hashq-ref active value)
            (unsupported 'cyclic-result)
            (begin
              (hashq-set! active value #t)
              (let ((copy
                     (if (pair? value)
                         ;; Flat lists consume the node budget, not nesting depth.
                         (cons (walk (car value) (+ depth 1))
                               (walk (cdr value) depth))
                         (let ((length (vector-length value)))
                           (when (> length (- %ipc-data-max-nodes nodes))
                             (throw 'ipc-data-error 'result-too-large))
                           (let ((copy (make-vector length)))
                             (do ((i 0 (+ i 1))) ((= i length) copy)
                               (vector-set! copy i
                                            (walk (vector-ref value i)
                                                  (+ depth 1)))))))))
                (hashq-remove! active value)
                copy))))
       (else (unsupported 'unreadable-result))))
    (walk value 0)))

(define (ipc-writable-datum value)
  "Sanitizes diagnostic/event data without invoking opaque object printers."
  (catch 'ipc-data-error
    (lambda () (ipc-normalize-datum value #t))
    (lambda (_ code) (string-append "<" (symbol->string code) ">"))))

(define (ipc-print-datum value)
  "Writes normalized VALUE to a bounded port; throws on limit overflow."
  (ipc-capture-output (lambda (port) (write value port))
                      %ipc-reply-max-chars #f))

(define (ipc-format-message key arguments)
  "Formats already sanitized exception data with bounded diagnostic output."
  (catch #t
    (lambda ()
      (ipc-capture-output
       (lambda (port)
         (if (and (pair? arguments) (pair? (cdr arguments))
                  (string? (cadr arguments)))
             (let ((subr (car arguments))
                   (message (cadr arguments))
                   (message-args (if (pair? (cddr arguments))
                                     (caddr arguments) '())))
               (when subr (format port "~a: " subr))
               ;; Only interpret the basic Guile condition directives. An
               ;; arbitrary format width or iteration is not a printing budget.
               (let loop ((i 0) (args (if (list? message-args) message-args '())))
                 (when (< i (string-length message))
                   (let ((char (string-ref message i)))
                     (if (and (char=? char #\~)
                              (< (+ i 1) (string-length message)))
                         (let ((directive (string-ref message (+ i 1))))
                           (cond
                            ((and (memv directive '(#\a #\A #\s #\S))
                                  (pair? args))
                             ((if (memv directive '(#\a #\A)) display write)
                              (car args) port)
                             (loop (+ i 2) (cdr args)))
                            ((char=? directive #\%)
                             (newline port) (loop (+ i 2) args))
                            ((char=? directive #\~)
                             (write-char #\~ port) (loop (+ i 2) args))
                            (else (write-char char port) (loop (+ i 1) args))))
                         (begin (write-char char port) (loop (+ i 1) args)))))))
             (format port "~a ~s" key arguments)))
       %ipc-message-max-chars #t))
    (lambda _ "exception (diagnostic formatting failed)")))

(define (ipc-format-backtrace stack)
  "Formats at most the configured frames and characters of STACK."
  (catch #t
    (lambda ()
      (if stack
          (ipc-capture-output
           (lambda (port)
             ;; Avoid rendering frame arguments: they can contain arbitrary
             ;; custom printers, cycles, or enormous values. Names and source
             ;; locations are sufficient for a useful bounded backtrace.
             (do ((i 1 (+ i 1)))
                 ((>= i (min (stack-length stack)
                             (+ 1 %ipc-backtrace-max-frames))))
               (let ((frame (stack-ref stack i)))
                 (format port "~a: ~s ~s~%" i
                         (ipc-writable-datum (frame-procedure-name frame))
                         (ipc-writable-datum (frame-source frame))))))
           %ipc-backtrace-max-chars #t)
          ""))
    (lambda _ "")))

(define (ipc-error-metadata stage)
  `((stage . ,stage)
    (execution-started? . ,(not (eq? stage 'validation)))
    (execution-completed? . ,(eq? stage 'result-encoding))
    (effects-may-have-occurred? . ,(not (eq? stage 'validation)))))

(define (ipc-error-reply-string key arguments message backtrace stage)
  "Preserves the five legacy fields, then adds explicit execution metadata."
  (let ((metadata (ipc-error-metadata stage)))
    (catch 'ipc-data-error
      (lambda ()
        (ipc-print-datum (list 'error key arguments message backtrace metadata)))
      (lambda _
        ;; A large error argument list cannot hide the failure stage. Retain the
        ;; bounded condition message, key, and metadata, dropping only irritants.
        (ipc-print-datum
         (list 'error (if (and (symbol? key)
                              (<= (string-length (symbol->string key)) 128))
                         key 'evaluation-error)
               '("<error arguments exceeded reply budget>")
               message backtrace metadata))))))

(define (ipc-ok-reply-string result)
  "Normalizes RESULT and emits one complete readable datum, or an encoding
error that explicitly records successful evaluation and possible effects."
  (catch 'ipc-data-error
    (lambda ()
      (ipc-print-datum (list 'ok (ipc-normalize-datum result))))
    (lambda (_ code)
      (ipc-error-reply-string
       code '()
       (case code
         ((cyclic-result) "evaluation completed; result contains a cycle")
         ((result-too-large) "evaluation completed; result exceeds the reply budget")
         ((result-too-deep) "evaluation completed; result exceeds the nesting limit")
         (else "evaluation completed; result contains an unsupported value"))
       "" 'result-encoding))))

(define (ipc-ok-reply result)
  (with-fluids ((read-eval? #f))
    (call-with-input-string (ipc-ok-reply-string result) read)))

(define (minde-ipc-eval source)
  "Evaluates one datum on the compositor thread. Parsing failures precede
execution; all other exceptions conservatively report possible side effects."
  (let ((stack #f) (stage 'validation))
    (catch #t
      (lambda ()
        (with-throw-handler #t
          (lambda ()
            (let ((datum
                   (with-fluids ((read-eval? #f))
                    (call-with-input-string source
                     (lambda (input)
                       (let ((value (read input)))
                         (when (eof-object? value)
                           (error "IPC requires one datum"))
                         (unless (eof-object? (read input))
                           (error "IPC accepts exactly one datum"))
                         value))))))
              (set! stage 'execution)
              (let ((result (eval datum (interaction-environment))))
                (set! stage 'result-encoding)
                (ipc-ok-reply-string result))))
          (lambda _ (set! stack (make-stack #t)))))
      (lambda (key . arguments)
        (let* ((safe-key (ipc-writable-datum key))
               (safe-arguments (ipc-writable-datum arguments))
               (details (ipc-format-message safe-key safe-arguments)))
          ;; Diagnostics must never prevent delivery of the actual reply.
          (catch #t
            (lambda () (wm-log (string-append "IPC failure: " details)))
            (lambda _ #f))
          (ipc-error-reply-string safe-key safe-arguments details
                                  (ipc-format-backtrace stack) stage))))))

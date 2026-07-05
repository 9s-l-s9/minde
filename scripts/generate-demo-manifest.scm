#!/bin/sh
exec guile --no-auto-compile -L scheme -s "$0" "$@"
!#
;;; SPDX-License-Identifier: GPL-3.0-or-later

(use-modules (ice-9 match)
             (srfi srfi-1)
             (minde commands)
             (minde command-catalog))

(define (read-one path)
  (call-with-input-file path
    (lambda (port)
      (let ((datum (read port)))
        (unless (eof-object? (read port))
          (error "scenario file contains trailing data" path))
        datum))))

(define (field name scenario)
  (let ((entry (assq name (cdr scenario))))
    (unless (and entry (= (length entry) 2))
      (error "scenario field must occur exactly once" name scenario))
    (cadr entry)))

(define data (read-one "demos/scenarios.scm"))
(unless (and (list? data)
             (eq? (car data) 'minde-demo-scenarios)
             (equal? (cadr data) '(version 1)))
  (error "unsupported demo scenario schema" data))

(define scenarios (cddr data))
(for-each
 (lambda (scenario)
   (unless (and (list? scenario) (eq? (car scenario) 'scenario))
     (error "invalid demo scenario" scenario))
   (unless (symbol? (field 'id scenario))
     (error "demo id must be a symbol" scenario))
   (for-each (lambda (name)
               (unless (string? (field name scenario))
                 (error "demo field must be a string" name scenario)))
             '(title trigger setup action cleanup))
   (unless (and (list? (field 'input scenario))
                (every string? (field 'input scenario)))
     (error "demo input must be a list of xdotool key names" scenario)))
 scenarios)

(define ids (map (lambda (scenario) (field 'id scenario)) scenarios))
(unless (= (length ids) (length (delete-duplicates ids)))
  (error "duplicate demo id" ids))

(register-builtin-command-schemas!)
(define catalog-ids
  (delete-duplicates
   (map (lambda (name) (command-demo-id (command-ref name)))
        (command-names))))
(unless (and (lset= eq? ids catalog-ids)
             (= (length ids) (length catalog-ids)))
  (error "scenario IDs and command demo IDs differ" ids catalog-ids))

(define (commands-for id)
  (filter (lambda (name)
            (eq? id (command-demo-id (command-ref name))))
          (command-names)))

(define (json-string value)
  (call-with-output-string
    (lambda (port)
      (write-char #\" port)
      (string-for-each
       (lambda (character)
         (case character
           ((#\") (display "\\\"" port))
           ((#\\) (display "\\\\" port))
           ((#\newline) (display "\\n" port))
           ((#\return) (display "\\r" port))
           ((#\tab) (display "\\t" port))
           (else (write-char character port))))
       value)
      (write-char #\" port))))

(define (json-array strings)
  (string-append "["
                 (string-join (map json-string strings) ",")
                 "]"))

(define tsv? (member "--tsv" (command-line)))

(if tsv?
    (for-each
     (lambda (scenario)
       (format #t "~a\t~a\t~a\t~a\t~a\t~a\t~a~%"
               (field 'id scenario)
               (field 'title scenario)
               (field 'trigger scenario)
               (if (null? (field 'input scenario))
                   "-"
                   (string-join (field 'input scenario) " "))
               (field 'setup scenario)
               (field 'action scenario)
               (field 'cleanup scenario)))
     scenarios)
    (begin
      (display "{\"schema_version\":1,\"generator\":\"scripts/generate-demo-manifest.scm\",\"scenarios\":[")
      (let loop ((remaining scenarios) (first? #t))
        (unless (null? remaining)
          (let* ((scenario (car remaining))
                 (id (field 'id scenario))
                 (stem (symbol->string id)))
            (unless first? (display ","))
            (format #t "{\"demo_id\":~a,\"commands\":~a,\"title\":~a,\"trigger\":~a,\"input\":~a,\"scenario\":\"demos/scenarios.scm\",\"video\":~a,\"poster\":~a,\"transcript\":~a}"
                    (json-string stem)
                    (json-array (map symbol->string (commands-for id)))
                    (json-string (field 'title scenario))
                    (json-string (field 'trigger scenario))
                    (json-array (field 'input scenario))
                    (json-string (string-append stem ".webm"))
                    (json-string (string-append stem ".png"))
                    (json-string (string-append stem ".txt")))
            (loop (cdr remaining) #f))))
      (display "]}\n")))

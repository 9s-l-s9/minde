;;; SPDX-License-Identifier: GPL-3.0-or-later
(use-modules (ice-9 regex)
             (srfi srfi-1)
             (minde commands)
             (minde command-catalog)
             (minde frames))

(define failures 0)
(define (check name value)
  (unless value
    (set! failures (+ failures 1))
    (format #t "FAIL - ~a~%" name)))

(register-builtin-command-schemas!)
(define public-modules
  '((minde windows)
    (minde frames)
    (minde groups)
    (minde layouts)
    (minde input)
    (minde commands)
    (minde hooks)
    (minde status)))
(for-each
 (lambda (module-name)
   (check (format #f "public module ~s resolves" module-name)
          (resolve-interface module-name)))
 public-modules)
(check "private compositor frame implementation resolves"
       (resolve-module '(minde compositor frames) #:ensure #f))
(define frame-interface (resolve-interface '(minde frames)))
(define grouped-frame-bindings (append-map cdr frame-api-groups))
(define public-frame-operations
  (filter (lambda (name) (not (memq name '(frame-api-groups frame-api-tags))))
          (module-map (lambda (name variable) name) frame-interface)))
(define (symbol-name<? left right)
  (string<? (symbol->string left) (symbol->string right)))
(check "frame API has eight named capability groups"
       (= 8 (length frame-api-groups)))
(check "frame API groups contain no duplicate bindings"
       (= (length grouped-frame-bindings)
          (length (delete-duplicates grouped-frame-bindings))))
(check "frame API groups classify every public operation exactly once"
       (equal? (sort grouped-frame-bindings symbol-name<?)
               (sort public-frame-operations symbol-name<?)))
(check "curated frame facade has the frozen 65-operation surface"
       (= 65 (length public-frame-operations)))
(for-each
 (lambda (entry)
   (check (format #f "frame API tag ~a has no duplicate bindings" (car entry))
          (= (length (cdr entry)) (length (delete-duplicates (cdr entry)))))
   (check (format #f "frame API tag ~a names only public operations" (car entry))
          (every (lambda (name) (memq name public-frame-operations)) (cdr entry))))
 frame-api-tags)
(check "catalog is non-empty" (pair? (command-names)))
(for-each
 (lambda (name)
   (let ((command (command-ref name)))
     (check (format #f "~a is canonical" name)
            (string-match "^[a-z][a-z0-9]*(-[a-z0-9]+)*[!?]?$"
                          (symbol->string name)))
     (check (format #f "~a has arguments" name) (list? (command-arguments command)))
     (check (format #f "~a has category" name) (symbol? (command-category command)))
     (check (format #f "~a has summary" name)
            (and (string? (command-summary command))
                 (not (string-null? (command-summary command)))))
     (check (format #f "~a has documentation" name)
            (and (string? (command-documentation command))
                 (not (string-null? (command-documentation command)))))
     (check (format #f "~a has demo id" name) (symbol? (command-demo-id command)))))
 (command-names))

(define forbidden-api-names
  '(gnext! gprev! gother! gnew! gnewbg! gkill! gmerge!
    snext! sprev! sother! fclear! only! sibling! banish!
    focus-prev-frame! focus-prev-window!))
(for-each
 (lambda (module-name)
   (let ((interface (resolve-interface module-name)))
     (for-each
      (lambda (name)
        (check (format #f "~s does not export ~a" module-name name)
               (not (module-variable interface name))))
      forbidden-api-names)))
 '((minde frames) (minde groups)))

(for-each
 (lambda (name)
   (check (format #f "public frame facade hides compositor internal ~a" name)
          (not (module-variable (resolve-interface '(minde frames)) name))))
 '(activate-group! current-group current-tree frame-add-window! hide-window!
   heads-changed! park-group-windows! set-sync-hook! sync-frames!
   track-float-map! track-window-map! track-window-unmap!
   update-floating-window-geometry! update-output-geometry!))

(if (zero? failures)
    (format #t "all API contract tests passed~%")
    (exit 1))

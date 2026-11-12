;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Files and registries survive failed loads and writes; boot errors surface.

(use-modules (srfi srfi-1) (ice-9 textual-ports)
             (minde foundation serialization))

(define %messages '())
(define %logs '())
(define %outputs '())
(define (wm-log text) (set! %logs (cons text %logs)))
(define (wm-message text . _) (set! %messages (cons text %messages)))
(define (wm-outputs) %outputs)
(define (wm-place-window . _) #t)
(define (wm-focus-window . _) #t)
(define (wm-focus-rect . _) #t)
(define (wm-clear-focus) #t)

(use-modules (minde compositor frames) (minde layouts) (minde groups))

(define %dir (format #f "/tmp/minde-persistence-test-~a" (getpid)))
(mkdir %dir #o700)
(define %layouts (string-append %dir "/layouts.scm"))
(define %rules (string-append %dir "/rules.scm"))
(setenv "MINDE_LAYOUTS_FILE" %layouts)
(setenv "MINDE_RULES_FILE" %rules)
(define %failures 0)
(define (check label actual expected)
  (if (equal? actual expected)
      (format #t "ok - ~a~%" label)
      (begin (set! %failures (+ 1 %failures))
             (format #t "FAIL - ~a: expected ~s, got ~s~%" label expected actual))))
(define (check-true label value) (check label (and value #t) #t))
(define (put path text) (call-with-output-file path (lambda (p) (display text p))))
(define (bytes path) (call-with-input-file path get-string-all))
(define (rules) (@@ (minde groups) %placement-rules))

(check "missing layouts are harmless" (load-layouts!) #t)
(check "missing rules are harmless" (load-placement-rules!) #t)
(check "missing files are quiet" (list %messages %logs) '(() ()))

;; Both failures occur before the first Rust output exists.
(put %layouts "(minde-layouts 2 ((\"old\" . leaf)))\n")
(put %rules "(minde-placement-rules 2 ())\n")
(check "unsupported layout version rejected" (load-layouts!) #f)
(check "unsupported rules version rejected" (load-placement-rules!) #f)
(check "boot errors wait for an output" %messages '())
(check-true "boot errors log both filenames"
            (and (any (lambda (s) (string-contains s %layouts)) %logs)
                 (any (lambda (s) (string-contains s %rules)) %logs)))
(handle-heads-change! '())
(check "empty heads do not consume pending errors" %messages '())
(set! %outputs '((0 0 0 1280 720)))
(handle-heads-change! %outputs)
(check "first output shows one combined diagnostic" (length %messages) 1)
(check-true "visible diagnostic names both preserved files"
            (and (string-contains (car %messages) %layouts)
                 (string-contains (car %messages) %rules)))
(handle-output-geometry! 0 0 1280 720)
(check "subsequent output changes do not repeat errors" (length %messages) 1)

(define-layout! "configured" 'leaf)
(handle-window-map! 1 "terminal" "foot")
(add-placement-rule! "foot")
(define %original-rules (rules))
(define %original-layouts (layout-names))

(for-each
 (lambda (text)
   (put %layouts text)
   (check "bad layouts fail to load" (load-layouts!) #f)
   (check "bad layouts block saving" (save-layout! "lost") #f)
   (check "failed layout operations preserve file bytes" (bytes %layouts) text)
   (check "bad layouts are never partially published" (layout-names) %original-layouts)
   (check-true "failed save shows its error"
               (string-contains (car %messages) "could not save")))
 '("(minde-layouts 2 ((\"old\" . leaf)))\n"
   "(minde-layouts 1 ("
   "(minde-layouts 1 ()) trailing\n"
   "(minde-layouts 1 ((\"partial\" . leaf) (\"bad\" hsplit 2 leaf leaf)))\n"
   "(minde-layouts 1 ((\"bad\" hsplit +nan.0 leaf leaf)))\n"
   "(minde-layouts 1 ((\"bad\" hsplit 1+2i leaf leaf)))\n"
   "(minde-layouts 1 not-a-list)\n"))

(for-each
 (lambda (text)
   (put %rules text)
   (check "bad rules fail to load" (load-placement-rules!) #f)
   (check "bad rules block remember" (remember!) #f)
   (check "failed remember preserves rules" (rules) %original-rules)
   (check "bad rules block forget" (forget!) #f)
   (check "failed forget preserves rules" (rules) %original-rules)
   (check "failed rule operations preserve file bytes" (bytes %rules) text)
   (check-true "failed forget does not claim success"
               (string-contains (car %messages) "could not save")))
 '("(minde-placement-rules 2 ())\n"
   "(minde-placement-rules 1 ("
   "(minde-placement-rules 1 ()) trailing\n"
   "(minde-placement-rules 1 ((\"partial\" #f 0 #f) (\"bad\")))\n"
   "(minde-placement-rules 1 ((\"bad\" #f 1.5 #f)))\n"
   "(minde-placement-rules 1 ((\"bad\" #f -1 #f)))\n"
   "(minde-placement-rules 1 ((\"bad\" #f 0 yes)))\n"
   "(minde-placement-rules 1 not-a-list)\n"))

;; Repairing a file must be followed by an explicit successful reload.
(write-versioned-datum-file %layouts 'minde-layouts 1 '(("repaired" . leaf)))
(check "repaired layout still requires reload" (save-layout! "retry") #f)
(check "repaired layout loads" (load-layouts!) #t)
(check "reload re-enables layout saves" (save-layout! "retry") #t)
(write-versioned-datum-file %rules 'minde-placement-rules 1 %original-rules)
(check "repaired rules still require reload" (remember!) #f)
(check "repaired rules load" (load-placement-rules!) #t)
(check "reload re-enables rule saves" (remember!) #t)

;; Every save validates the disk file, even after an earlier successful load.
(put %layouts "external invalid edit\n")
(check "external bad layout edit blocks save" (save-layout! "external") #f)
(check "external layout edit is preserved" (bytes %layouts) "external invalid edit\n")
(delete-file %layouts)
(check "moving a bad layout file aside permits a new save" (save-layout! "new") #t)
(put %rules "external invalid edit\n")
(check "external bad rules edit blocks save" (save-placement-rules!) #f)
(check "external rules edit is preserved" (bytes %rules) "external invalid edit\n")
(delete-file %rules)
(check "moving a bad rules file aside permits a new save" (save-placement-rules!) #t)

;; Force an atomic-write failure without depending on process permissions.
(for-each
 (lambda (path operation snapshot)
   (let ((before (bytes path)) (memory (snapshot))
         (temp (format #f "~a.tmp.~a" path (getpid))))
     (mkdir temp)
     (check "failed write returns false" (operation) #f)
     (check "failed write preserves original file" (bytes path) before)
     (check "failed write preserves registry" (snapshot) memory)
     (rmdir temp)))
 (list %layouts %rules %rules)
 (list (lambda () (save-layout! "unwritten")) remember! forget!)
 (list layout-names rules rules))

;; A dangling link is an existing unreadable path, not an absent state file.
(delete-file %layouts)
(symlink (string-append %dir "/missing-target") %layouts)
(check "dangling layout link fails load" (load-layouts!) #f)
(check "dangling layout link blocks replacement" (save-layout! "link") #f)
(check "dangling link is preserved" (stat:type (lstat %layouts)) 'symlink)
(setenv "MINDE_LAYOUTS_FILE" (string-append %dir "/other.scm"))
(check "failure at one path does not block another" (save-layout! "other") #t)
(delete-file (getenv "MINDE_LAYOUTS_FILE"))

;; Legacy equivalent rules normalize once; distinct policies keep their order.
(write-versioned-datum-file
 %rules 'minde-placement-rules 1
 '(("foot" #f 0 #f) ("foot" #f 0 #f #t) ("foot" #f 0 #f #t #f)
   ("foot" "II" 0 #f #t #f) ("Picture-in-Picture" #f 0 #f #t #t)
   ("Picture-in-Picture" #f 0 #f #t #t)))
(check "legacy rules load" (load-placement-rules!) #t)
(check "equivalent legacy rules deduplicate without merging policies" (rules)
       '(("foot" #f 0 #f #t #f) ("foot" "II" 0 #f #t #f)
         ("Picture-in-Picture" #f 0 #f #t #t)))

(let ((cycle (list 'hsplit 1/2 'leaf 'leaf)))
  (set-car! (cddr cycle) cycle)
  (check "cyclic layout trees are rejected" (valid-layout-spec? cycle) #f))
(delete-file %layouts)
(delete-file %rules)
(rmdir %dir)
(format #t "~a persistence failure(s)~%" %failures)
(exit (if (zero? %failures) 0 1))

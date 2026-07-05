;;; groups.scm -- StumpWM-style groups (workspaces), each with its own
;;; frame tree.
;;;
;;; Owns the ordered list of groups and "which one is current" (delegated
;;; to (minde frames)'s notion of the active group -- see
;;; activate-group! there). Higher-level operations (switching, creating
;;; groups, moving a window between groups) live here; the frame-tree
;;; internals stay in (minde frames).
;;;
;;; Same load-time constraint as frames.scm (see its header comment):
;;; nothing here may call a wm-* Rust subr at module load time. Creating
;;; the default groups below is pure data construction (make-empty-group,
;;; set-group-name!) and must stay that way.

(define-module (minde groups)
  #:use-module (srfi srfi-1)
  #:use-module (ice-9 optargs)
  #:use-module (minde hooks)
  #:use-module (minde frames)
  #:use-module (minde compositor model)
  #:use-module (minde foundation serialization)
  #:re-export (focus-next-head! focus-previous-head!)
  #:export (switch-to-group!
            switch-to-last-group!
            add-placement-rule!
            clear-placement-rules!
            place-existing-windows!
            status-line
            switch-to-next-group!
            switch-to-previous-group!
            create-group!
            create-auto-named-group!
            create-floating-group!
            create-group-in-background!
            create-floating-group-in-background!
            shift-current-window-to-next-group!
            shift-current-window-to-previous-group!
            merge-group-into-current!
            delete-other-groups!
            move-marked-windows-to-group!
            kill-windows-current-group!
            kill-windows-other!
            groups-echo-string
            remember!
            forget!
            save-placement-rules!
            load-placement-rules!
            dump-desktop
            dump-desktop-to-file
            restore-from-file
            handle-window-move!
            rename-current-group!
            delete-current-group!
            move-window-to-next-group!
            move-current-window-to-next-group-and-follow!
            next-urgent!
            handle-heads-change!
            set-head-mode!
            handle-window-map!
            handle-window-title-change!
            handle-window-unmap!
            handle-output-geometry!
            current-group-name
            group-names
            group-has-window?
            dynamic-group?
            mark-dynamic!
            create-dynamic-group!
            create-dynamic-group-in-background!
            retile-dynamic!
            retile-dynamic-group!
            rotate-windows!
            rotate-stack!
            exchange-with-master!
            change-layout!
            change-split-ratio!
            change-default-layout!
            change-default-split-ratio!))

;; ---------------------------------------------------------------------
;; The group list
;; ---------------------------------------------------------------------

;; Ordered list of all <group>s. The active one (per (current-group) in
;; frames.scm) is always a member of this list.
(define %groups '())

(define (init-default-groups!)
  ;; frames.scm already bootstrapped one active group wrapping its
  ;; initial %frame-tree/%current-frame; adopt it as " I " rather than
  ;; building a redundant fresh one, so there's only ever one "the active
  ;; group" object.
  (let ((g1 (current-group)))
    (set-group-name! g1 " I ")
    (set! %groups (list g1
                        (make-empty-group " II " 1280 720)
                        (make-empty-group " III " 1280 720)))))

(init-default-groups!)

(define (group-names) (map group-name %groups))

(define (current-group-name) (group-name (current-group)))

(define (find-group-by-name name)
  (find (lambda (g) (string=? (group-name g) name)) %groups))

(define (current-group-index)
  (list-index (lambda (g) (eq? g (current-group))) %groups))

(define (resolve-target name-or-index)
  (cond
   ((string? name-or-index) (find-group-by-name name-or-index))
   ((integer? name-or-index)
    (and (>= name-or-index 0) (< name-or-index (length %groups))
         (list-ref %groups name-or-index)))
   (else #f)))

;; True if group NAME currently tracks window ID (mapped or not, on- or
;; off-screen) -- for tests/inspection.
(define (group-has-window? name id)
  (let ((g (find-group-by-name name)))
    (and g (member id (group-window-ids g)) #t)))

(define (group-window-ids g)
  (append (append-map frame-window-ids
                      (append-map frame-leaves (group-all-trees g)))
          (group-floats g)))

;; ---------------------------------------------------------------------
;; Switching
;; ---------------------------------------------------------------------

(define (switch-to-group! name-or-index)
  "Switches to the group named NAME-OR-INDEX (a string) or at index
NAME-OR-INDEX (0-based). No-op if it doesn't exist or is already
current. Parks every window of the outgoing group off-screen, then
activates the target group and syncs it (placing its windows, restoring
or clearing focus)."
  (let ((target (resolve-target name-or-index)))
    (when (and target (not (eq? target (current-group))))
      (set! %last-group (current-group))
      ;; Always-show windows follow the switch: pull each one into the
      ;; target group (from whichever group holds it) before parking.
      (for-each
       (lambda (id)
         (let ((g (find (lambda (g) (member id (group-window-ids g)))
                        %groups)))
           (when (and g (not (eq? g target)))
             (move-window-between-groups! id g target))))
       (sticky-windows))
      (park-group-windows! (current-group))
      (activate-group! target)
      (sync-frames!)
      ;; A dynamic group re-derives its master/stack tiling on
      ;; activation (gmoves into it while hidden left the tree stale).
      (when (dynamic-group? target) (retile-dynamic!))
      (run-event-hook! 'focus-group (group-name target))
      (echo (group-list-string)))))

;; The group that was current before the last switch (StumpWM gother).
(define %last-group #f)

(define (switch-to-last-group!)
  "Toggles back to the previously current group."
  (when (and %last-group (memq %last-group %groups))
    (switch-to-group! (group-name %last-group))))

(define (group-list-string)
  "The group list with the current one bracketed, StumpWM message style:
\"[ I ]  II   III \"."
  (string-join
   (map (lambda (g)
          (if (eq? g (current-group))
              (string-append "[" (string-trim-both (group-name g)) "]")
              (string-trim-both (group-name g))))
        %groups)
   "  "))

(define (switch-to-next-group!)
  "Switches to the next group in %groups, wrapping around."
  (let ((idx (current-group-index)) (n (length %groups)))
    (when (and idx (> n 1))
      (switch-to-group! (modulo (+ idx 1) n)))))

(define (switch-to-previous-group!)
  "Switches to the previous group in %groups, wrapping around."
  (let ((idx (current-group-index)) (n (length %groups)))
    (when (and idx (> n 1))
      (switch-to-group! (modulo (- idx 1) n)))))

;; ---------------------------------------------------------------------
;; Creating groups
;; ---------------------------------------------------------------------

(define (create-group-in-background! name)
  "Creates a new empty group named NAME (a string), appended to the end of
%groups, sized to the last known output geometry. Does not switch to it
(StumpWM gnewbg)."
  (let* ((size (current-output-size))
         (g (make-empty-group name (car size) (cadr size))))
    (set! %groups (append %groups (list g)))
    g))

(define (create-group! name)
  "Creates a new empty group named NAME and switches to it (StumpWM
gnew)."
  (let ((g (create-group-in-background! name)))
    (switch-to-group! (group-name g))
    g))

;; Roman numerals for auto-generated group names (I, II, III already
;; taken by the defaults; create-auto-named-group! continues IV, V, ...). Falls back to
;; a plain number past 3999, which will never happen in practice.
(define %roman-table
  '((1000 . "M") (900 . "CM") (500 . "D") (400 . "CD")
    (100 . "C") (90 . "XC") (50 . "L") (40 . "XL")
    (10 . "X") (9 . "IX") (5 . "V") (4 . "IV") (1 . "I")))

(define (integer->roman n)
  (let loop ((n n) (table %roman-table) (acc ""))
    (cond
     ((zero? n) acc)
     ((null? table) (number->string n)) ; shouldn't happen for sane n
     ((>= n (caar table)) (loop (- n (caar table)) table (string-append acc (cdar table))))
     (else (loop n (cdr table) acc)))))

(define (create-auto-named-group!)
  "Creates a new group with an auto-generated roman-numeral name (\" IV \",
\" V \", ... continuing the default groups' naming) and switches to it."
  (create-group! (string-append " " (integer->roman (+ 1 (length %groups))) " ")))

(define (create-floating-group-in-background! name)
  "Creates a new float group in the background (StumpWM gnewbg-float):
every window mapped while it is current floats automatically. Does not
switch to it."
  (let ((g (create-group-in-background! name)))
    (set-group-float?! g #t)
    g))

(define (create-floating-group! name)
  "Creates a new float group and switches to it (StumpWM gnew-float)."
  (let ((g (create-floating-group-in-background! name)))
    (switch-to-group! (group-name g))
    g))

(define (rename-current-group! name)
  "Renames the current group (StumpWM grename). Group names are stored
padded (\" I \"); NAME is padded the same way."
  (unless (string-null? name)
    (set-group-name! (current-group) (string-append " " (string-trim-both name) " "))
    (echo (group-list-string))))

(define (delete-current-group!)
  "Deletes the current group; its windows move to the previous (or next)
group, which becomes current (StumpWM gkill). A no-op with one group."
  (let ((g (current-group)) (n (length %groups)))
    (when (> n 1)
      (let* ((idx (current-group-index))
             (fallback (list-ref %groups (modulo (+ idx 1) n)))
             (ids (group-window-ids g)))
        ;; Adopt the doomed group's windows into the fallback group's
        ;; current frame (numbers re-uniquified there), parked off-screen
        ;; until that group is shown.
        (for-each
         (lambda (id)
           ;; Floats of the doomed group are unfloated into the fallback
           ;; group's frame like everything else.
           (unless (remove-float! g id)
             (find (lambda (t) (remove-window-from-tree-in! t id))
                   (group-all-trees g)))
           (hide-window! id)
           (ensure-unique-window-number! id (group-all-trees fallback))
           (frame-add-window! (group-current-frame fallback) id))
         ids)
        (switch-to-group! (group-name fallback))
        (set! %groups (delq g %groups))
        (unmark-dynamic! g)
        (when (eq? %last-group g) (set! %last-group #f))
        (echo (group-list-string))))))

;; ---------------------------------------------------------------------
;; Moving windows between groups
;; ---------------------------------------------------------------------

(define (move-window-to-next-group!)
  "Moves the current group's current window into the next group's current
frame, and hides it (it leaves the screen; the current group stays
current -- this does not follow the window). A no-op if there's only one
group or the current frame has no window."
  (let ((idx (current-group-index)) (n (length %groups)))
    (when (and idx (> n 1))
      (let ((id (focused-window-id)))
        (when id
          (move-window-between-groups!
           id (current-group) (list-ref %groups (modulo (+ idx 1) n)))
          (sync-frames!))))))

(define (gshift-with-window! dir)
  (let ((idx (current-group-index)) (n (length %groups))
        (id (focused-window-id)))
    (when (and idx (> n 1) id)
      (let ((target (list-ref %groups (modulo (+ idx dir) n))))
        (move-window-between-groups! id (current-group) target)
        (switch-to-group! (group-name target))
        (focus-window-by-id! id)))))

(define (shift-current-window-to-next-group!)
  "Moves the current window to the next group and follows it (StumpWM
gnext-with-window)."
  (gshift-with-window! 1))

(define (shift-current-window-to-previous-group!)
  "Moves the current window to the previous group and follows it (StumpWM
gprev-with-window)."
  (gshift-with-window! -1))

;; Adopts every window of G into the current group and deletes G. The
;; shared core of merge-group-into-current! and delete-other-groups!; callers sync and echo.
(define (adopt-group-into-current! g)
  (when (and (not (eq? g (current-group))) (memq g %groups))
    (for-each
     (lambda (id) (move-window-between-groups! id g (current-group)))
     (group-window-ids g))
    (set! %groups (delq g %groups))
    (unmark-dynamic! g)
    (when (eq? %last-group g) (set! %last-group #f))))

(define (merge-group-into-current! name-or-index)
  "Merges the group named NAME-OR-INDEX into the current one: all its
windows (floats stay floating) move here and the emptied group is
deleted (StumpWM gmerge)."
  (let ((g (resolve-target name-or-index)))
    (cond
     ((not g) (echo "no such group"))
     ((eq? g (current-group)) (echo "already the current group"))
     (else
      (adopt-group-into-current! g)
      (sync-frames!)
      (echo (group-list-string))))))

(define (delete-other-groups!)
  "Deletes every group except the current one, merging all their windows
into it (StumpWM gkill-other)."
  (for-each adopt-group-into-current! (list-copy %groups))
  (sync-frames!)
  (echo (group-list-string)))

(define (move-marked-windows-to-group! name-or-index)
  "Moves every marked window of the current group into the given group
and clears their marks (StumpWM gmove-marked)."
  (let ((target (resolve-target name-or-index)))
    (if (or (not target) (eq? target (current-group)))
        (echo "no such group")
        (let ((ids (filter (lambda (id)
                             (member id (group-window-ids (current-group))))
                           (marked-windows))))
          (if (null? ids)
              (echo "no marked windows")
              (begin
                (for-each
                 (lambda (id)
                   (move-window-between-groups! id (current-group) target)
                   (unmark-window! id))
                 ids)
                (sync-frames!)
                (echo (format #f "moved ~a window(s) to~a"
                              (length ids) (group-name target)))))))))

;; ---------------------------------------------------------------------
;; Group-wide window closing
;; ---------------------------------------------------------------------

(define (kill-windows-current-group!)
  "Politely closes every window of the current group (StumpWM
kill-windows-current-group)."
  (let ((ids (group-window-ids (current-group))))
    (for-each (lambda (id) (rust-call 'wm-close-window id)) ids)
    (echo (format #f "closing ~a window(s)" (length ids)))))

(define (kill-windows-other!)
  "Politely closes every window of every group except the current one
(StumpWM kill-windows-other)."
  (let ((n 0))
    (for-each
     (lambda (g)
       (unless (eq? g (current-group))
         (for-each (lambda (id)
                     (set! n (+ n 1))
                     (rust-call 'wm-close-window id))
                   (group-window-ids g))))
     %groups)
    (echo (format #f "closing ~a window(s)" n))))

(define (groups-echo-string)
  "The group list one per line with window counts, the current group
marked * (StumpWM groups/vgroups)."
  (string-join
   (map (lambda (g)
          (format #f "~a~a (~a)"
                  (if (eq? g (current-group)) "*" " ")
                  (string-trim-both (group-name g))
                  (group-window-count g)))
        %groups)
   "\n"))

;; Same dynamic (guile-user) lookup as frames.scm's rust-call (see the
;; long comment there): a missing subr (old binary, test stubs) is a
;; no-op.
(define (rust-call name . args)
  (let* ((mod (resolve-module '(guile-user) #:ensure #f))
         (var (and mod (module-variable mod name))))
    (when var (apply (variable-ref var) args))))

;; Parks a float off-screen without disturbing its remembered geometry
;; (hide-window! would place it with tiled states).
(define (rust-call-place-float-offscreen id r)
  (rust-call 'wm-place-float id -10000 -10000
             (if r (caddr r) 100) (if r (cadddr r) 100)))

;; The workhorse behind every window-between-groups operation (gmove,
;; gmerge, sticky windows, gnext-with-window): moves ID from group FROM
;; into TO, keeping float status and geometry, parked until TO is
;; shown. Callers sync/switch as appropriate.
(define (move-window-between-groups! id from to)
  (unless (eq? from to)
    (if (window-floating? id)
        (when (member id (group-floats from))
          (let ((r (float-geometry id)))
            (set-group-floats! from (delete id (group-floats from)))
            (set-group-floats! to (cons id (group-floats to)))
            (rust-call-place-float-offscreen id r)))
        (when (find (lambda (t) (remove-window-from-tree-in! t id))
                    (group-all-trees from))
          ;; The active group's record fields can be stale between
          ;; flushes; target the live current frame instead.
          (if (eq? to (current-group))
              (frame-add-window! (current-frame) id)
              (begin
                (hide-window! id)
                (frame-add-window! (group-current-frame to) id)))))
    (ensure-unique-window-number! id (group-all-trees to))
    ;; If either end is the visible group and dynamic, retile it now;
    ;; a hidden dynamic group retiles when activated.
    (when (and (eq? to (current-group)) (dynamic-group? to))
      (retile-dynamic!))
    (when (and (eq? from (current-group)) (dynamic-group? from))
      (retile-dynamic!))))

;; ---------------------------------------------------------------------
;; Dynamic groups (StumpWM dynamic-group.lisp): master/stack auto-tiling
;; as a group property. The newest window is the master at
;; ratio-of-the-head in the layout position ('left default); everything
;; else stacks evenly in the remainder. Retiled on map/unmap/gmove/
;; float toggles; manual splits are refused (see init.scm's guard).
;; State is per group AND head: group -> (head-id -> (order layout
;; ratio)), order master-first.
;; ---------------------------------------------------------------------

(define %dynamic-groups (make-hash-table)) ; hashq <group> -> head table
(define %dynamic-default-layout 'left)     ; 'left 'right 'top 'bottom
(define %dynamic-default-ratio 2/3)        ; StumpWM default-split-ratio

(define* (dynamic-group? #:optional (g (current-group)))
  (and (hashq-ref %dynamic-groups g) #t))

(define (mark-dynamic! g)
  (hashq-set! %dynamic-groups g (make-hash-table)))

(define (unmark-dynamic! g)
  (hashq-remove! %dynamic-groups g))

(define (dynamic-state g hid)
  "The (order layout ratio) list for G on head HID, created on first
use. #f when G isn't dynamic."
  (let ((ht (hashq-ref %dynamic-groups g)))
    (and ht
         (or (hashv-ref ht hid)
             (let ((s (list '() %dynamic-default-layout
                            %dynamic-default-ratio)))
               (hashv-set! ht hid s)
               s)))))

(define (set-dynamic-state! g hid order layout ratio)
  (hashv-set! (hashq-ref %dynamic-groups g) hid (list order layout ratio)))

(define (dynamic-dump order layout ratio)
  "A dump-frames-shaped snapshot placing ORDER master-first in the
master/stack layout: (spec leaf-window-lists leaf-currents
master-leaf-index)."
  (let ((n (length order)))
    (if (< n 2)
        (list 'leaf (list order)
              (list (and (pair? order) (car order))) 0)
        (let* ((stack (chain-spec (if (memq layout '(left right))
                                      'vertical 'horizontal)
                                  (- n 1)))
               (spec (case layout
                       ((left) (list 'hsplit ratio 'leaf stack))
                       ((right) (list 'hsplit (- 1 ratio) stack 'leaf))
                       ((top) (list 'vsplit ratio 'leaf stack))
                       (else (list 'vsplit (- 1 ratio) stack 'leaf))))
               ;; frame-leaves order is depth-first, so the master leaf
               ;; is first for left/top and last for right/bottom.
               (lists (if (memq layout '(left top))
                          (map list order)
                          (append (map list (cdr order))
                                  (list (list (car order)))))))
          (list spec lists (map car lists)
                (if (memq layout '(left top)) 0 (- n 1)))))))

(define (retile-dynamic!)
  "Recomputes the current head's master/stack tiling for the current
group: windows that appeared since the last retile become the master
(newest first), vanished ones drop out. No-op in a manual group."
  (let* ((g (current-group))
         (hid (current-head-id))
         (st (dynamic-state g hid)))
    (when st
      (let* ((tiled (apply append (cadr (dump-frames))))
             (kept (filter (lambda (id) (member id tiled)) (car st)))
             (order (append (filter (lambda (id) (not (member id kept)))
                                    tiled)
                            kept))
             (layout (cadr st))
             (ratio (caddr st)))
        (set-dynamic-state! g hid order layout ratio)
        (restore-frames! (dynamic-dump order layout ratio))))))

(define (create-dynamic-group-in-background! name)
  "Creates a dynamic (auto-tiling) group in the background (StumpWM
gnewbg-dynamic)."
  (let ((g (create-group-in-background! name)))
    (mark-dynamic! g)
    g))

(define (create-dynamic-group! name)
  "Creates a dynamic (auto-tiling) group and switches to it (StumpWM
gnew-dynamic)."
  (let ((g (create-dynamic-group-in-background! name)))
    (switch-to-group! (group-name g))
    g))

(define (reorder-dynamic! f)
  "Applies F to the current head's window order and retiles. Echoes in
a manual group."
  (let ((st (dynamic-state (current-group) (current-head-id))))
    (if (not st)
        (echo "not a dynamic group")
        (let ((order (f (car st))))
          (set-dynamic-state! (current-group) (current-head-id)
                              order (cadr st) (caddr st))
          (restore-frames! (dynamic-dump order (cadr st) (caddr st)))))))

(define (rotate-list lst dir)
  (cond ((or (null? lst) (null? (cdr lst))) lst)
        ((eq? dir 'backward) (append (cdr lst) (list (car lst))))
        (else (cons (last lst) (list-head lst (- (length lst) 1))))))

(define* (rotate-windows! #:optional (dir 'forward))
  "Rotates all windows through the master/stack positions (StumpWM
rotate-windows)."
  (reorder-dynamic! (lambda (order) (rotate-list order dir))))

(define* (rotate-stack! #:optional (dir 'forward))
  "Rotates only the stack windows, master stays (StumpWM rotate-stack)."
  (reorder-dynamic!
   (lambda (order)
     (if (< (length order) 3)
         order
         (cons (car order) (rotate-list (cdr order) dir))))))

(define (exchange-with-master!)
  "Swaps the focused window with the master (StumpWM
exchange-with-master)."
  (let ((cur (focused-window-id)))
    (if (not cur)
        (echo "no focused window")
        (reorder-dynamic!
         (lambda (order)
           (if (or (null? order) (equal? cur (car order)))
               order
               (cons cur (delete cur order))))))))

(define (change-dynamic-param! setter)
  (let* ((g (current-group))
         (hid (current-head-id))
         (st (dynamic-state g hid)))
    (if (not st)
        (echo "not a dynamic group")
        (begin
          (apply set-dynamic-state! g hid (setter st))
          (retile-dynamic!)))))

(define %dynamic-layouts '(left right top bottom))

(define (change-layout! sym)
  "Sets the master position for this group+head (StumpWM change-layout):
left, right, top or bottom."
  (if (memq sym %dynamic-layouts)
      (change-dynamic-param!
       (lambda (st) (list (car st) sym (caddr st))))
      (echo (format #f "no layout ~a (left right top bottom)" sym))))

(define (change-split-ratio! r)
  "Sets the master size as a fraction of the head for this group+head
(StumpWM change-split-ratio)."
  (let ((r (and (number? r) (max 1/10 (min 9/10 r)))))
    (if (not r)
        (echo "ratio must be a number")
        (change-dynamic-param!
         (lambda (st) (list (car st) (cadr st) r))))))

(define (change-default-layout! sym)
  "Sets the default master position for new dynamic group heads."
  (when (memq sym %dynamic-layouts)
    (set! %dynamic-default-layout sym)))

(define (change-default-split-ratio! r)
  "Sets the default master ratio for new dynamic group heads."
  (when (number? r)
    (set! %dynamic-default-ratio (max 1/10 (min 9/10 r)))))

(define (retile-dynamic-group!)
  "Forces a retile of the current dynamic group's head (StumpWM
retile)."
  (if (dynamic-group?)
      (retile-dynamic!)
      (echo "not a dynamic group")))

;; ---------------------------------------------------------------------
;; Placement rules (StumpWM define-frame-preference): route a newly
;; mapped window to a specific group and/or frame by matching its app-id
;; or title.
;; ---------------------------------------------------------------------

;; Each rule: (matcher group-name frame-index follow? lock?). MATCHER
;; is a string matched as a substring against the window's app-id, then
;; its title. GROUP-NAME is compared trimmed (" II " and "II" both
;; work); #f means "the current group". FRAME-INDEX is 0-based in tree
;; order, clamped to the existing leaves. First matching rule wins.
;; LOCK? #t (the default, StumpWM's :lock) applies the rule when the
;; window maps; #f rules only fire through place-existing-windows!.
(define %placement-rules '())

(define* (add-placement-rule! matcher #:key (group #f) (frame 0)
                              (follow? #f) (raise? #f) (lock? #t))
  ;; #:raise? is StumpWM's name for what our #:follow? does.
  (set! %placement-rules
        (append %placement-rules
                (list (list matcher group frame (or follow? raise?) lock?)))))

(define (rule-lock? rule)
  ;; Rules loaded from an old 4-element file default to locked.
  (or (< (length rule) 5) (list-ref rule 4)))

(define (clear-placement-rules!)
  (set! %placement-rules '()))

;; ---------------------------------------------------------------------
;; Rule persistence + remember/forget (StumpWM remember / forget /
;; dump-window-placement-rules / restore-window-placement-rules) --
;; same file pattern as (minde layouts).
;; ---------------------------------------------------------------------

(define (rules-file)
  (or (getenv "MINDE_RULES_FILE")
      (string-append (or (getenv "HOME") ".") "/.config/minde/rules.scm")))

(define (save-placement-rules!)
  "Writes the placement rules to the rules file."
  (let ((path (rules-file)))
    (catch #t
      (lambda ()
        (let ((dir (dirname path)))
          (unless (file-exists? dir) (mkdir dir)))
        (write-versioned-datum-file
         path 'minde-placement-rules 1 %placement-rules))
      (lambda (key . args)
        (echo (format #f "could not save rules: ~a ~s" key args))))))

(define (load-placement-rules!)
  "Replaces the placement rules with the rules file's contents. Quietly
does nothing if the file is missing or unreadable."
  (let ((path (rules-file)))
    (when (file-exists? path)
      (catch #t
        (lambda ()
          (let ((saved (read-versioned-datum-file
                        path 'minde-placement-rules 1)))
            (when (list? saved)
              (set! %placement-rules
                    (filter (lambda (r) (and (pair? r) (string? (car r))))
                            saved)))))
        (lambda (key . args)
          (echo (format #f "could not load rules: ~a ~s" key args)))))))

(define (remember!)
  "Adds a persistent placement rule pinning the focused window's
app-id (or title) to its current group and frame (StumpWM remember)."
  (let ((id (focused-window-id)))
    (if (not id)
        (echo "no window")
        (let ((matcher (or (window-app-id id) (window-title id)))
              (frame-idx (or (list-index (lambda (f) (eq? f (current-frame)))
                                         (frame-leaves (current-tree)))
                             0)))
          (add-placement-rule! matcher
                               #:group (string-trim-both (current-group-name))
                               #:frame frame-idx)
          (save-placement-rules!)
          (echo (format #f "remembered: ~a ->~a frame ~a"
                        matcher (current-group-name) frame-idx))))))

(define (forget!)
  "Drops every placement rule matching the focused window and persists
(StumpWM forget)."
  (let ((id (focused-window-id)))
    (if (not id)
        (echo "no window")
        (let* ((title (window-title id))
               (app (window-app-id id))
               (before (length %placement-rules)))
          (set! %placement-rules
                (remove (lambda (r) (rule-matches? r title app))
                        %placement-rules))
          (save-placement-rules!)
          (echo (format #f "forgot ~a rule(s)"
                        (- before (length %placement-rules))))))))

(define (rule-matches? rule title app-id)
  (let ((m (car rule)))
    (or (and (string? app-id) (string-contains app-id m))
        (and (string? title) (string-contains title m)))))

(define (find-group-loose name)
  (and name
       (find (lambda (g)
               (string=? (string-trim-both (group-name g))
                         (string-trim-both name)))
             %groups)))

(define (place-by-rule! rule id title app-id)
  (let* ((g (or (find-group-loose (cadr rule)) (current-group)))
         (active? (eq? g (current-group)))
         (tree (if active? (current-tree) (group-tree g)))
         (leaves (frame-leaves tree))
         (leaf (list-ref leaves (min (max 0 (caddr rule))
                                     (- (length leaves) 1)))))
    (remember-window-title! id title app-id)
    (assign-window-number! id (group-all-trees g))
    (frame-add-window! leaf id)
    (run-event-hook! 'new-window id title app-id)
    (if active?
        (sync-frames!)
        (begin
          ;; Park it: it belongs to a hidden group and must not linger
          ;; on-screen at whatever geometry it mapped with.
          (hide-window! id)
          (when (cadddr rule)
            (switch-to-group! (group-name g)))))))

(define (place-existing-windows!)
  "Re-applies the placement rules to every already-mapped window of the
active group (StumpWM place-existing-windows). Floats keep floating and
are left alone; a window already in its rule's target frame stays put."
  (let ((moved 0))
    (for-each
     (lambda (id)
       (unless (window-floating? id)
         (let* ((title (window-title id))
                (rule (find (lambda (r)
                              (rule-matches? r title (window-app-id id)))
                            %placement-rules)))
           (when (and rule (apply-rule-to-mapped! id rule))
             (set! moved (+ moved 1))))))
     (all-window-ids))
    (sync-frames!)
    (echo (format #f "placed ~a window(s)" moved))))

(define (apply-rule-to-mapped! id rule)
  "Moves an already-mapped window into RULE's group/frame (the shared
core of place-existing-windows! and late app-id arrival). Returns #t if
the window actually moved; the caller re-syncs."
  (let* ((g (or (find-group-loose (cadr rule)) (current-group)))
         (active? (eq? g (current-group)))
         (tree (if active? (current-tree) (group-tree g)))
         (leaves (frame-leaves tree))
         (leaf (list-ref leaves (min (max 0 (caddr rule))
                                     (- (length leaves) 1)))))
    (and (not (member id (frame-window-ids leaf)))
         (begin
           (find (lambda (gg)
                   (any (lambda (t) (remove-window-from-tree-in! t id))
                        (group-all-trees gg)))
                 %groups)
           (ensure-unique-window-number! id (group-all-trees g))
           (frame-add-window! leaf id)
           (unless active? (hide-window! id))
           #t))))

;; ---------------------------------------------------------------------
;; Desktop dump/restore (StumpWM dump-desktop-to-file /
;; restore-from-file): every group's name, float flag, frame layout
;; with window assignments (loaded head's tree), and float geometries.
;; Window ids are session-local -- restore is for the running session;
;; stale ids are dropped by restore-group-frames!.
;; ---------------------------------------------------------------------

(define (dump-desktop)
  (list 'minde-desktop 1
        (head-mode)
        (map (lambda (g)
               (list (group-name g)
                     (group-float? g)
                     (dump-group-frames g)
                     (map (lambda (id) (cons id (float-geometry id)))
                          (group-floats g))))
             %groups)))

(define (dump-desktop-to-file path)
  (catch #t
    (lambda ()
      (write-datum-file path (dump-desktop))
      (echo (string-append "desktop dumped to " path)))
    (lambda (key . args)
      (echo (format #f "could not dump desktop: ~a ~s" key args)))))

(define (restore-from-file path)
  "Re-applies a dump-desktop file: groups are matched (or created) by
name, their frame layouts and window assignments restored, float
geometries re-applied. Groups not in the dump are left alone."
  (catch #t
    (lambda ()
      (let ((d (call-with-input-file path read)))
        (if (not (and (list? d) (>= (length d) 4)
                      (eq? (car d) 'minde-desktop)
                      (equal? (cadr d) 1)))
            (echo (string-append path " is not a desktop dump"))
            (begin
              (for-each
               (lambda (entry)
                 (let* ((name (car entry))
                        (g (or (find-group-by-name name) (create-group-in-background! name))))
                   (set-group-float?! g (cadr entry))
                   (restore-group-frames! g (caddr entry))
                   (for-each
                    (lambda (fg)
                      (when (and (pair? (cdr fg))
                                 (member (car fg) (group-floats g)))
                        (set-float-geometry! (car fg) (cdr fg))))
                    (cadddr entry))))
               (cadddr d))
              (sync-frames!)
              (echo (string-append "desktop restored from " path))))))
    (lambda (key . args)
      (echo (format #f "could not restore desktop: ~a ~s" key args)))))

;; ---------------------------------------------------------------------
;; Status line for external bars (eww etc.): written to
;; $XDG_RUNTIME_DIR/minde-status whenever it changes, via the
;; frames.scm sync hook. Consume with `tail -F` (eww deflisten) or poll
;; `minde-cmd '(status-line)'`.
;; ---------------------------------------------------------------------

(define (status-line)
  (let ((id (focused-window-id)))
    (string-append (group-list-string)
                   " | "
                   (if id (window-title id) ""))))

(define %status-path
  (string-append (or (getenv "XDG_RUNTIME_DIR") "/tmp") "/minde-status"))

(define %last-status #f)

(define (write-status!)
  (let ((s (status-line)))
    (unless (equal? s %last-status)
      (set! %last-status s)
      (catch #t
        (lambda ()
          (call-with-output-file %status-path
            (lambda (port) (display s port) (newline port))))
        (lambda _ #f)))))

(set-sync-hook! write-status!)

(define (move-current-window-to-next-group-and-follow!)
  "Moves the current window to the next group and switches there with it
(StumpWM gmove-and-follow)."
  (let ((idx (current-group-index)) (n (length %groups))
        (id (focused-window-id)))
    (when (and idx (> n 1) id)
      (let ((next (list-ref %groups (modulo (+ idx 1) n))))
        (move-window-to-next-group!)
        (switch-to-group! (group-name next))
        (focus-window-by-id! id)))))

;; ---------------------------------------------------------------------
;; Rust-facing hooks (looked up by name in (guile-user) -- see
;; frames.scm's rust-call comment for why these must be plain top-level
;; procedures rather than something requiring qualified lookup).
;; ---------------------------------------------------------------------

(define (handle-window-map! id title app-id)
  (let ((rule (find (lambda (r) (and (rule-lock? r)
                                     (rule-matches? r title app-id)))
                    %placement-rules)))
    (if rule
        (place-by-rule! rule id title app-id)
        (track-window-map! id title app-id))
    ;; New tiled window in a dynamic group: it becomes the master.
    (when (and (dynamic-group?) (not (window-floating? id)))
      (retile-dynamic!))))

(define (handle-window-title-change! id title app-id)
  "Rust: a mapped toplevel's title/app-id changed. Wayland clients set
both only after the initial configure, so handle-window-map! usually saw
empty strings and the real values arrive here (and again on every
retitle). Refreshes the bookkeeping (windowlist, remapped keys, rules,
status line); when the app-id first becomes known, a lock placement
rule that missed the window at map time is applied now."
  (let ((first-app-id? (let ((old (window-app-id id)))
                         (and (or (not old) (string-null? old))
                              (not (string-null? app-id))))))
    (update-window-title! id title app-id)
    (when (and first-app-id? (not (window-floating? id)))
      (let ((rule (find (lambda (r) (and (rule-lock? r)
                                         (rule-matches? r title app-id)))
                        %placement-rules)))
        (when (and rule (apply-rule-to-mapped! id rule))
          (if (cadddr rule)   ; follow? flag
              (switch-to-group!
               (group-name (or (find-group-loose (cadr rule)) (current-group))))
              (sync-frames!)))))
    ;; External bars show the focused window's title.
    (write-status!)))

(define (next-urgent!)
  "Jumps to the oldest urgent window (StumpWM next-urgent), switching to
its group if needed."
  (let ((urgent (urgent-windows)))
    (if (null? urgent)
        (echo "No urgent windows")
        (let* ((id (car urgent))
               (g (find (lambda (g) (group-has-window? (group-name g) id))
                        %groups)))
          (clear-urgent! id)
          (cond
           ((not g) (next-urgent!)) ; window vanished meanwhile: try the next
           (else
            (unless (eq? g (current-group))
              (switch-to-group! (group-name g)))
            (focus-window-by-id! id)))))))

(define (handle-window-unmap! id)
  "Removes ID from whichever group's tree currently holds it -- the active
group first (which re-syncs), then every hidden group (which doesn't need
a sync since nothing hidden is on-screen)."
  (clear-fullscreen-if-window! id)
  (clear-urgent! id)
  (clear-ontop! id)
  (clear-sticky! id)
  (unmark-window! id)
  (clear-unmaximized! id)
  (let ((fg (find (lambda (g) (remove-float! g id)) %groups)))
    (cond
     (fg (when (eq? fg (current-group)) (sync-frames!)))
     ((remove-window-from-active-tree! id) #t)
     (else
      (find (lambda (g)
              (any (lambda (t) (remove-window-from-tree-in! t id))
                   (group-all-trees g)))
            %groups))))
  (forget-window-title! id)
  (forget-window-number! id)
  ;; A dynamic group refills the master/stack arrangement.
  (when (dynamic-group?) (retile-dynamic!))
  (run-event-hook! 'destroy-window id)
  #t)

(define (handle-window-move! id x y w h)
  "Rust reports the final geometry of a super+drag move/resize."
  (update-floating-window-geometry! id x y w h))

(define (handle-output-geometry! x y width height)
  "Single-head compatibility path (old binaries / winit-era configs)."
  (when (and (> width 0) (> height 0))
    (heads-changed! (list (list 0 x y width height)) %groups)))

(define (handle-heads-change! heads)
  "Multi-head backends report the full usable-rect list here:
((id x y w h) ...)."
  (heads-changed! heads %groups))

(define (set-head-mode! mode)
  "'per-head (a frame tree per monitor, StumpWM style) or 'span (one
tree over the union of all monitors)."
  (set-heads-mode! mode %groups))

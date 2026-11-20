;;; groups.scm -- StumpWM-style groups (workspaces), each with its own
;;; frame tree.
;;;
;;; Owns the ordered list of groups and "which one is current" (delegated
;;; to (minde compositor frames)'s notion of the active group -- see
;;; activate-group! there). Higher-level operations (switching, creating
;;; groups, moving a window between groups) live here; the frame-tree
;;; internals stay in (minde compositor frames).
;;;
;;; Same load-time constraint as frames.scm (see its header comment):
;;; nothing here may call a wm-* Rust subr at module load time. Creating
;;; the default groups below is pure data construction (make-empty-group,
;;; set-group-name!) and must stay that way.

(define-module (minde groups)
  #:use-module (srfi srfi-1)
  #:use-module (ice-9 optargs)
  #:use-module (minde hooks)
  #:use-module (minde compositor frames)
  #:use-module (minde compositor model)
  #:use-module (minde compositor revision)
  #:use-module (minde compositor rust)
  #:use-module (minde foundation serialization)
  #:use-module (minde persistence)
  #:use-module (minde placement)
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
            handle-window-fullscreen!
            handle-foreign-activate!
            rename-current-group!
            delete-current-group!
            move-window-to-next-group!
            move-current-window-to-next-group-and-follow!
            next-urgent!
            handle-heads-change!
            set-head-mode!
            output-heads
            configure-output!
            output-configuration-allowed?
            handle-output-configured!
            handle-output-configure-failed!
            keyboard-layouts
            set-keyboard-layout!
            handle-keyboard-layout-changed!
            handle-window-map!
            handle-window-title-change!
            handle-window-unmap!
            handle-output-geometry!
            current-group-name
            group-names
            group-status-summaries
            current-layout-status
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
            change-default-split-ratio!
            wm-on-session-lock
            wm-on-session-lock-lost
            wm-on-session-unlock))

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
(set-gap-groups-provider! (lambda () %groups))

(define (group-names)
  "Returns every group name in switching order."
  (map group-name %groups))

(define (current-group-name)
  "Returns the padded name of the active group."
  (group-name (current-group)))

(define (find-group-by-name name)
  (find (lambda (g) (string=? (group-name g) name)) %groups))

(define (current-group-index)
  (list-index (lambda (g) (eq? g (current-group))) %groups))

(define (resolve-target name-or-index)
  (cond
   ((group? name-or-index) (and (memq name-or-index %groups) name-or-index))
   ((string? name-or-index) (find-group-by-name name-or-index))
   ((exact-integer? name-or-index)
    (and (>= name-or-index 0) (< name-or-index (length %groups))
         (list-ref %groups name-or-index)))
   (else #f)))

;; True if group NAME currently tracks window ID (mapped or not, on- or
;; off-screen) -- for tests/inspection.
(define (group-has-window? name id)
  "Returns true when group NAME contains window ID in a tree or float list."
  (let ((g (find-group-by-name name)))
    (and g (member id (group-window-ids g)) #t)))

(define (window-group id)
  (find (lambda (g) (member id (group-window-ids g))) %groups))

(define (distinct-window-count ids)
  ;; Mirrored heads can contain the same ID. Count without constructing a
  ;; deduplicated list or repeatedly comparing each ID with the whole tail.
  (let ((seen (make-hash-table)) (count 0))
    (for-each (lambda (id)
                (unless (hash-ref seen id)
                  (hash-set! seen id #t)
                  (set! count (+ count 1))))
              ids)
    count))

(define (group-status-summaries)
  "Returns stable status tuples for every group.
Each tuple is (NAME FOCUSED? WINDOW-COUNT FLOATING? DYNAMIC?)."
  (map (lambda (group)
         (list (group-name group)
               (eq? group (current-group))
               (distinct-window-count (group-window-ids group))
               (group-float? group)
               (dynamic-group? group)))
       %groups))

;; ---------------------------------------------------------------------
;; Switching
;; ---------------------------------------------------------------------

(define (switch-to-group! name-or-index)
  "Switches to the group named NAME-OR-INDEX (a string), at that index
(0-based), or supplied as a live group record. No-op for an unknown name,
out-of-range index or deleted group record, or when the group is already
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
         (let ((g (window-group id)))
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
    (switch-to-group! %last-group)))

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

(define (validate-group-name name except)
  (unless (and (string? name) (not (string-null? (string-trim-both name))))
    (error "group name must be a nonempty string" name))
  (when (any (lambda (g)
               (and (not (eq? g except))
                    (string=? (string-trim-both name)
                              (string-trim-both (group-name g)))))
             %groups)
    (error "group name already exists" (string-trim-both name))))

(define (create-group-in-background! name)
  "Creates a new empty group named NAME (a string), appended to the end of
%groups, sized to the last known output geometry. Does not switch to it
(StumpWM gnewbg). Rejects blank or duplicate names before changing state;
surrounding whitespace is ignored when checking uniqueness."
  (validate-group-name name #f)
  (let* ((size (current-output-size))
         (g (make-empty-group name (car size) (cadr size))))
    (set! %groups (append %groups (list g)))
    (note-policy-change!)
    g))

(define (create-group! name)
  "Creates a new empty group named NAME and switches to it (StumpWM
gnew)."
  (let ((g (create-group-in-background! name)))
    (switch-to-group! g)
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
  "Creates and switches to a group with the first unused roman-numeral
name, starting at I. With the default groups present, starts at IV."
  (let loop ((n 1))
    (let ((name (string-append " " (integer->roman n) " ")))
      (if (find-group-loose name)
          (loop (+ n 1))
          (create-group! name)))))

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
    (switch-to-group! g)
    g))

(define (rename-current-group! name)
  "Renames the current group (StumpWM grename). Group names are stored
padded (\" I \"); NAME is padded the same way. Rejects another group's name,
ignoring surrounding whitespace. Blank input leaves the name unchanged."
  (unless (and (string? name) (string-null? (string-trim-both name)))
    (validate-group-name name (current-group))
    (set-group-name! (current-group) (string-append " " (string-trim-both name) " "))
    (note-policy-change!)
    (echo (group-list-string))))

(define (delete-current-group!)
  "Deletes the current group; its windows move to the next group, which
becomes current, wrapping from the last group to the first (StumpWM gkill).
A no-op with one group."
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
           (ensure-unique-window-number! id fallback)
           (frame-add-window! (group-current-frame fallback) id))
         ids)
        (switch-to-group! fallback)
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
        (switch-to-group! target)
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

;; Parks a float off-screen without disturbing its remembered geometry
;; (hide-window! would place it with tiled states).
(define (rust-call-place-float-offscreen id r)
  (rust-place-float! id -10000 -10000
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
    (ensure-unique-window-number! id to)
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
  "Returns true when G is managed by the dynamic master/stack policy."
  (and (hashq-ref %dynamic-groups g) #t))

(define (mark-dynamic! g)
  "Marks G dynamic and creates its empty per-head policy table."
  (hashq-set! %dynamic-groups g (make-hash-table))
  (note-policy-change!))

(define (unmark-dynamic! g)
  (hashq-remove! %dynamic-groups g)
  (note-policy-change!))

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

(define* (dynamic-dump order layout ratio #:optional selected)
  "A dump-frames-shaped snapshot placing ORDER master-first in the
master/stack layout: (spec leaf-window-lists leaf-currents
selected-leaf-index). Selects SELECTED when present, otherwise the master."
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
                (or (and selected (list-index (lambda (ids) (member selected ids)) lists))
                    (if (memq layout '(left top)) 0 (- n 1))))))))

(define (dynamic-frame-dump g hid dump)
  (let* ((st (dynamic-state g hid))
         (tiled (apply append (cadr dump)))
         (kept (filter (lambda (id) (member id tiled)) (car st)))
         (order (append (filter (lambda (id) (not (member id kept))) tiled) kept))
         (layout (cadr st))
         (ratio (caddr st)))
    (set-dynamic-state! g hid order layout ratio)
    (dynamic-dump order layout ratio (list-ref (caddr dump) (cadddr dump)))))

(define (retile-dynamic!)
  "Recomputes the current head's master/stack tiling for the current
group: windows that appeared since the last retile become the master
(newest first), vanished ones drop out. Preserves the selected tiled
window when it survives, otherwise selects the master. No-op in a manual group."
  (when (dynamic-group?)
    (restore-frames! (dynamic-frame-dump (current-group) (current-head-id) (dump-frames)))))

(define (retile-group-heads! g)
  ;; Window mutations can affect a head other than the focused one, even
  ;; in a hidden group. Rebuild those trees without borrowing input focus.
  (when (dynamic-group? g)
    (map-group-frame-trees! g (lambda (hid dump) (dynamic-frame-dump g hid dump)))
    (when (eq? g (current-group)) (sync-frames!))))

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
    (switch-to-group! g)
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
exchange-with-master). Does nothing unless focus belongs to the current
head's tiled order."
  (let* ((cur (focused-window-id))
         (st (dynamic-state (current-group) (current-head-id))))
    (cond
     ((not cur) (echo "no focused window"))
     ((not st) (echo "not a dynamic group"))
     ((not (member cur (car st))) (echo "focused window is not tiled on this head"))
     (else
      (reorder-dynamic!
       (lambda (order)
         (if (equal? cur (car order))
             order
             (cons cur (delete cur order)))))))))

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

(define (current-layout-status)
  "Returns a stable description of the current group/head layout.
Dynamic layouts return (dynamic POSITION RATIO); manual layouts return
(manual SPEC), where SPEC is the public frame-tree layout datum."
  (if (dynamic-group?)
      (let ((state (dynamic-state (current-group) (current-head-id))))
        (list 'dynamic (cadr state) (caddr state)))
      (list 'manual (dump-layout-spec))))

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

;; Each rule: (matcher group-name frame-index follow? lock? float?). MATCHER
;; contains separate exact app-id and substring title predicates, combined
;; with AND. GROUP-NAME is compared trimmed (" II " and "II" both
;; work); #f means "the current group". FRAME-INDEX is 0-based in tree
;; order, clamped to the existing leaves. First matching rule wins.
;; LOCK? #t (the default, StumpWM's :lock) applies the rule when the
;; window maps; #f rules only fire through place-existing-windows!.
(define %placement-rules '())

(define* (add-placement-rule! #:optional (matcher #f)
                              #:key (app-id #f) (title #f) (group #f) (frame 0)
                              (follow? #f) (raise? #f) (lock? #t)
                              (float? #f))
  "Appends a placement rule unless an equivalent rule already exists.
#:app-id matches exactly; #:title is a case-sensitive substring. When both
are supplied, both must match. The deprecated positional string matches
an exact app-id, falling back to a title substring only without an app-id.
#:float? #t makes matching windows float instead of joining a frame --
e.g. Firefox's Picture-in-Picture player, which would otherwise hijack
the current frame and push the browser window offscreen."
  ;; #:raise? is StumpWM's name for what our #:follow? does.
  (let ((rule (list (make-placement-matcher matcher app-id title)
                    group frame (or follow? raise?) lock? float?)))
    (unless (valid-placement-rules? (list rule)) (error "invalid placement rule" rule))
    (unless (member rule %placement-rules)
      (let* ((candidate (append %placement-rules (list rule)))
             (new-warnings (lset-difference string=?
                            (placement-rule-warnings candidate)
                            (placement-rule-warnings %placement-rules))))
        (set! %placement-rules candidate)
        (for-each (lambda (warning) (rust-call-if-bound 'wm-log warning)) new-warnings)))))

(define (rule-lock? rule)
  (list-ref rule 4))

(define (rule-float? rule)
  (list-ref rule 5))

(define (clear-placement-rules!)
  "Removes every in-memory placement rule without writing the rules file."
  (set! %placement-rules '()))

;; ---------------------------------------------------------------------
;; Rule persistence + remember/forget (StumpWM remember / forget /
;; dump-window-placement-rules / restore-window-placement-rules) --
;; same file pattern as (minde layouts).
;; ---------------------------------------------------------------------

(define (rules-file)
  (or (getenv "MINDE_RULES_FILE")
      (string-append (or (getenv "HOME") ".") "/.config/minde/rules.scm")))

(define (persist-placement-rules! candidate)
  (let ((saved (save-persistent-state!
                (rules-file) 'minde-placement-rules valid-placement-rules?
                (lambda (_)
                  (unless (valid-placement-rules? candidate)
                    (error "invalid placement rules" candidate))
                  (normalize-placement-rules candidate))
                #:version placement-rules-version #:migrate migrate-placement-rules)))
    (and saved (begin (set! %placement-rules saved) #t))))

(define (save-placement-rules!)
  "Writes deduplicated placement rules atomically. Returns #f on failure,
preserving unreadable files and leaving the in-memory rules unchanged."
  (persist-placement-rules! %placement-rules))

(define (load-placement-rules!)
  "Loads fully validated rules, migrating version 1 and deduplicating.
Migration uses exact app-id matches with title fallback only without an
app-id. The file is upgraded to version 2 on the next successful save.
Missing files are ignored. Invalid files preserve memory and block saves
until a successful reload or removal. Returns #t on success."
  (load-persistent-state!
   (rules-file) 'minde-placement-rules valid-placement-rules?
   (lambda (saved)
     (set! %placement-rules (normalize-placement-rules saved))
     (for-each (lambda (warning) (rust-call-if-bound 'wm-log warning))
               (placement-rule-warnings %placement-rules)))
   #:version placement-rules-version #:migrate migrate-placement-rules))

(define (remember!)
  "Pins the focused window's exact app-id to its current group and frame.
Refuses windows without an app-id instead of remembering a transient title."
  (let ((id (focused-window-id)))
    (if (not id)
        (echo "no window")
        (let ((app-id (window-app-id id))
              (frame-idx (or (list-index (lambda (f) (eq? f (current-frame)))
                                         (frame-leaves (current-tree)))
                             0)))
          (if (not (and (string? app-id) (not (string-null? app-id))))
              (begin (echo "cannot remember: window has no app-id") #f)
              (and (persist-placement-rules!
                (append %placement-rules
                        (list (list `((app-id . ,app-id)) (string-trim-both (current-group-name))
                                    frame-idx #f #t #f))))
               (begin
                 (echo (format #f "remembered: ~a ->~a frame ~a"
                               app-id (current-group-name) frame-idx))
                 #t)))))))

(define (forget!)
  "Drops every placement rule matching the focused window and persists
(StumpWM forget)."
  (let ((id (focused-window-id)))
    (if (not id)
        (echo "no window")
        (let* ((title (window-client-title id))
               (app (window-app-id id))
               (before (length %placement-rules)))
          (and (persist-placement-rules!
                (remove (lambda (r) (rule-matches? r title app))
                        %placement-rules))
               (begin
                 (echo (format #f "forgot ~a rule(s)"
                               (- before (length %placement-rules))))
                 #t))))))

(define (window-in-active-group? id)
  "True when ID is tiled in one of the active group's frame trees."
  (any (lambda (tree)
         (any (lambda (leaf) (and (member id (frame-window-ids leaf)) #t))
              (frame-leaves tree)))
       (group-all-trees (current-group))))

(define (rule-matches? rule title app-id)
  (placement-rule-matches? rule title app-id))

(define (find-group-loose name)
  (and name
       (find (lambda (g)
               (string=? (string-trim-both (group-name g))
                         (string-trim-both name)))
             %groups)))

(define (place-by-rule! rule id title app-id)
  (let* ((g (or (find-group-loose (cadr rule)) (current-group)))
         (active? (eq? g (current-group)))
         (float? (or (group-float? g) (rule-float? rule)))
         (tree (if active? (current-tree) (group-tree g)))
         (leaves (frame-leaves tree))
         (leaf (list-ref leaves (min (max 0 (caddr rule))
                                     (- (length leaves) 1)))))
    (remember-window-title! id title app-id)
    (assign-window-number! id g)
    (if float?
        (add-float! g id (default-float-rect))
        (frame-add-window! leaf id))
    (run-event-hook! 'new-window id title app-id)
    (if active?
        (if float? (focus-window-by-id! id) (sync-frames!))
        (begin
          ;; Park it: it belongs to a hidden group and must not linger
          ;; on-screen at whatever geometry it mapped with.
          (if float?
              (rust-call-place-float-offscreen id (float-geometry id))
              (hide-window! id))
          (when (cadddr rule)
            (switch-to-group! g)
            (when float? (focus-window-by-id! id)))))))

(define (place-existing-windows!)
  "Re-applies the placement rules to every already-mapped window of the
active group (StumpWM place-existing-windows). Floats keep floating and
are left alone; a window already in its rule's target frame stays put.
Float rules and floating target groups float matching tiled windows.
Affected dynamic groups retile every head immediately."
  (let ((moved 0))
    (for-each
     (lambda (id)
       (unless (window-floating? id)
         (let* ((title (window-client-title id))
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
  (let* ((source (window-group id))
         (g (or (find-group-loose (cadr rule)) (current-group)))
         (active? (eq? g (current-group)))
         (float? (or (group-float? g) (rule-float? rule)))
         (tree (if active? (current-tree) (group-tree g)))
         (leaves (frame-leaves tree))
         (leaf (list-ref leaves (min (max 0 (caddr rule))
                                     (- (length leaves) 1)))))
    (and source (or float? (not (member id (frame-window-ids leaf))))
         (begin
           (any (lambda (t) (remove-window-from-tree-in! t id)) (group-all-trees source))
           (ensure-unique-window-number! id g)
           (if float?
               (begin
                 (clear-unmaximized! id)
                 (add-float! g id (default-float-rect))
                 (if active?
                     (focus-window-by-id! id)
                     (rust-call-place-float-offscreen id (float-geometry id))))
               (begin
                 (frame-add-window! leaf id)
                 (unless active? (hide-window! id))))
           (retile-group-heads! source)
           (unless (eq? source g) (retile-group-heads! g))
           #t))))

;; ---------------------------------------------------------------------
;; Desktop dump/restore (StumpWM dump-desktop-to-file /
;; restore-from-file): every group's name, float flag, frame layout
;; with window assignments (loaded head's tree), and float geometries.
;; Window ids are session-local -- restore is for the running session;
;; stale ids are dropped by restore-group-frames!.
;; ---------------------------------------------------------------------

(define (dump-desktop)
  "Returns the versioned running-session desktop snapshot datum."
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
  "Writes the running-session desktop snapshot to PATH and echoes the result."
  (catch #t
    (lambda ()
      (write-datum-file path (dump-desktop))
      (echo (string-append "desktop dumped to " path)))
    (lambda (key . args)
      (echo (format #f "could not dump desktop: ~a ~s" key args)))))

(define (window-id? value)
  (and (integer? value) (exact? value) (>= value 0)))

(define (valid-frame-dump? dump)
  ;; (spec leaf-window-lists leaf-currents current-frame-index)
  (and (list? dump) (= (length dump) 4)
       (valid-layout-spec? (car dump))
       (let ((window-lists (cadr dump)) (currents (caddr dump)))
         (and (list? window-lists)
              (every (lambda (ids) (and (list? ids) (every window-id? ids)))
                     window-lists)
              ;; A window belongs to one frame.
              (let ((ids (concatenate window-lists)))
                (= (length ids) (length (delete-duplicates ids))))
              (list? currents)
              (every (lambda (id) (or (not id) (window-id? id))) currents)))
       (window-id? (cadddr dump))))

(define (valid-desktop-dump? datum)
  (define (valid-float? float)
    (and (pair? float) (window-id? (car float))
         (or (not (cdr float))
             (and (list? (cdr float)) (= (length (cdr float)) 4)
                  (every integer? (cdr float))))))
  (and (list? datum) (= (length datum) 4)
       (eq? (car datum) 'minde-desktop)
       (equal? (cadr datum) 1)
       (list? (cadddr datum))
       (every (lambda (entry)
                (and (list? entry) (= (length entry) 4)
                     (string? (car entry))
                     (boolean? (cadr entry))
                     (valid-frame-dump? (caddr entry))
                     (list? (cadddr entry))
                     (every valid-float? (cadddr entry))))
              (cadddr datum))))

(define (restore-from-file path)
  "Re-applies a dump-desktop file: groups are matched (or created) by
name, their frame layouts and window assignments restored, float
geometries re-applied. Groups not in the dump are left alone. The whole
file is validated first; a malformed one changes nothing."
  (catch #t
    (lambda ()
      (let ((d (call-with-input-file path read)))
        (if (not (valid-desktop-dump? d))
            (echo (string-append path " is not a desktop dump"))
            (begin
              (for-each
               (lambda (entry)
                 (let* ((name (car entry))
                        (g (or (find-group-loose name) (create-group-in-background! name))))
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
;; Compatibility text status for external bars. The versioned structured
;; publisher lives in (minde status) and installs the frames sync hook
;; from init.scm, avoiding a groups <-> status module cycle.
;; ---------------------------------------------------------------------

(define (status-line)
  "Returns the compatibility one-line group and focused-window status."
  (let ((id (focused-window-id)))
    (string-append (group-list-string)
                   " | "
                   (if id (window-title id) ""))))

(define (refresh-external-status!)
  (rust-call-if-bound 'publish-status!))

(define (move-current-window-to-next-group-and-follow!)
  "Moves the current window to the next group and switches there with it
(StumpWM gmove-and-follow)."
  (let ((idx (current-group-index)) (n (length %groups))
        (id (focused-window-id)))
    (when (and idx (> n 1) id)
      (let ((next (list-ref %groups (modulo (+ idx 1) n))))
        (move-window-to-next-group!)
        (switch-to-group! next)
        (focus-window-by-id! id)))))

;; ---------------------------------------------------------------------
;; Rust-facing hooks (looked up by name in (guile-user) -- see
;; frames.scm's rust-call comment for why these must be plain top-level
;; procedures rather than something requiring qualified lookup).
;; ---------------------------------------------------------------------

(define (handle-window-map! id title app-id)
  "Places a newly mapped window, applies locked rules, and retiles if dynamic."
  (let ((rule (find (lambda (r) (and (rule-lock? r)
                                     (rule-matches? r title app-id)))
                    %placement-rules)))
    (if rule
        (place-by-rule! rule id title app-id)
        (track-window-map! id title app-id))
    ;; New tiled window in a dynamic group: it becomes the master.
    (unless (window-floating? id)
      (let ((g (window-group id))) (when g (retile-group-heads! g))))))

(define (handle-window-title-change! id title app-id)
  "Rust: a mapped toplevel's title/app-id changed. Refreshes bookkeeping
(windowlist, remapped keys, rules, status line). If the app-id was empty
at the first buffer commit, apply any locked placement rule when it
first becomes known."
  ;; Rust already exposes the new values when this notification arrives.
  ;; Display titles can also be renamed or fall back to the app-id, so use
  ;; the previous raw notification rather than querying display metadata.
  (let* ((previous (update-window-title! id title app-id))
         (old-title (and previous (car previous)))
         (old-app-id (and previous (cdr previous)))
         (first-app-id? (and (or (not old-app-id) (string-null? old-app-id))
                             (not (string-null? app-id)))))
    ;; A client can set title/app-id after its first buffer commit, so a
    ;; Picture-in-Picture window may initially map with empty strings. Only
    ;; newly matching rules float it, so unfloating by hand isn't
    ;; overridden by later retitles.
    (when (and (not (window-floating? id))
               (window-in-active-group? id))
      (let ((rule (find (lambda (r) (and (rule-lock? r) (rule-float? r)
                                         (rule-matches? r title app-id)
                                         (not (rule-matches? r old-title
                                                             old-app-id))))
                        %placement-rules)))
        (when rule
          (float-window! id)
          (retile-group-heads! (current-group)))))
    (when (and first-app-id? (not (window-floating? id)))
      (let ((rule (find (lambda (r) (and (rule-lock? r)
                                         (not (rule-float? r))
                                         (rule-matches? r title app-id)))
                        %placement-rules)))
        (when (and rule (apply-rule-to-mapped! id rule))
          (let ((target (or (find-group-loose (cadr rule)) (current-group))))
            (if (and (cadddr rule) (not (eq? target (current-group))))
                (begin
                  (switch-to-group! target)
                  (when (window-floating? id) (focus-window-by-id! id)))
                (sync-frames!))))))
    ;; External bars show the focused window's title. A title-only update
    ;; does not otherwise require a frame synchronization.
    (refresh-external-status!)
    (unless (and (equal? old-title title) (equal? old-app-id app-id))
      (run-event-hook! 'window-title-changed id title app-id))))

(define (next-urgent!)
  "Jumps to the oldest urgent window (StumpWM next-urgent), switching to
its group if needed."
  (let ((urgent (urgent-windows)))
    (if (null? urgent)
        (echo "No urgent windows")
        (let* ((id (car urgent))
               (g (window-group id)))
          (clear-urgent! id)
          (cond
           ((not g) (next-urgent!)) ; window vanished meanwhile: try the next
           (else
            (unless (eq? g (current-group))
              (switch-to-group! g))
            (focus-window-by-id! id)))))))

(define (handle-window-fullscreen! id on)
  "Rust: an application requests fullscreen for ID without taking focus.
Records hidden-group requests for the next activation; unknown IDs do nothing."
  (let ((group (window-group id)))
    (and group (set-group-fullscreen! group id on))))

(define (handle-foreign-activate! id)
  "Rust: a taskbar requests activation. Validate ID before changing groups."
  (let ((group (window-group id)))
    (and group (group-owns-window? group id)
         (begin
           (unless (eq? group (current-group)) (switch-to-group! group))
           (when (and (fullscreen-window) (not (equal? (fullscreen-window) id)))
             (fullscreen!))
           (focus-window-by-id! id)))))

(define (handle-window-unmap! id)
  "Removes ID from whichever group's tree currently holds it -- the active
group first (which re-syncs), then every hidden group (which doesn't need
a sync since nothing hidden is on-screen)."
  (define owner (window-group id))
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
  ;; Repair the owning group, including its unfocused and hidden heads.
  (when owner (retile-group-heads! owner))
  (run-event-hook! 'destroy-window id)
  #t)

(define (handle-window-move! id x y w h)
  "Rust reports the final geometry of a super+drag move/resize."
  (update-floating-window-geometry! id x y w h))

(define (handle-output-geometry! x y width height)
  "Single-head compatibility path (old binaries / winit-era configs)."
  (when (and (> width 0) (> height 0))
    (heads-changed! (list (list 0 x y width height)) %groups)
    (show-persistence-errors!)))

(define (retile-changed-heads! before)
  ;; Returning borrowed windows removes members from their temporary host.
  ;; Dynamic groups must rebuild those frames, including hidden groups.
  ;; Geometry-only changes already resize trees in heads-changed!.
  (unless (lset= eqv? before (map car (heads)))
    (for-each retile-group-heads! %groups)))

(define (handle-heads-change! new-heads)
  "Multi-head backends report the full usable-rect list here:
((id x y w h) ...)."
  (note-policy-change!)
  (let ((before (map car (heads))))
    (heads-changed! new-heads %groups)
    (retile-changed-heads! before))
  (when (pair? new-heads) (show-persistence-errors!)))

(define (set-head-mode! mode)
  "'per-head (a frame tree per monitor, StumpWM style) or 'span (one
tree over the union of all monitors)."
  (let ((before (map car (heads))))
    (set-heads-mode! mode %groups)
    (retile-changed-heads! before)))

;; ---------------------------------------------------------------------
;; Output (monitor) configuration. Thin, keyword-argument wrappers over
;; the Rust primitives `wm-output-heads` and `wm-configure-output!`
;; (src/guile/mod.rs), plus the default definitions of the three
;; output-policy entry points Rust looks up by plain top-level name.
;; ---------------------------------------------------------------------

(define (output-heads)
  "Every known output head, disabled ones included, as a list of alists:
((name . \"DP-1\") (enabled . #t) (make . \"DEL\") (model . \"U2723QE\")
 (serial . \"ABC\") (description . \"DEL U2723QE ABC\")
 (mode . (3840 2160 60000)) (preferred-mode . (3840 2160 60000))
 (position . (1920 0)) (scale . 1.5) (transform . normal)
 (adaptive-sync . #f) (modes . ((3840 2160 60000) ...))).
Modes are (width height refresh-mhz); MODE and PREFERRED-MODE are #f
when unknown; ADAPTIVE-SYNC is #t, #f or the symbol `unsupported'.
Unlike `wm-outputs' (enabled heads' usable rectangles, keyed by the
compositor's head id) this is the raw per-connector view a
wlr-output-management client sees. Returns '() without a compositor."
  (let ((heads (rust-call 'wm-output-heads)))
    (if (list? heads) heads '())))

(define* (configure-output! name #:key (mode 'unset) (position 'unset)
                            (scale 'unset) (transform 'unset)
                            (enabled 'unset) (adaptive-sync 'unset))
  "Ask the compositor to reconfigure the head called NAME (a connector
name as listed by `output-heads'). Keyword settings, each left unchanged
when omitted:
  #:mode          (w h refresh-mhz), (w h) or a string \"WxH@R\" (R in Hz)
  #:position      (x y) in global logical coordinates
  #:scale         a positive number, e.g. 1.5
  #:transform     normal, 90, 180, 270, flipped, flipped-90, ... (symbol)
                  or the rotation in degrees
  #:enabled       #t or #f
  #:adaptive-sync #t or #f
Example: (configure-output! \"DP-1\" #:scale 1.5 #:position '(1920 0)).
An omitted or zero mode refresh chooses the preferred mode at that size,
otherwise the highest advertised refresh. To configure a disabled head,
include #:enabled #t; settings combined with #:enabled #f are rejected.
The change is queued to the compositor thread, validated (unknown head
or mode, unsupported adaptive sync, disabling the last head are refused)
and applied through the same revert-on-failure routine external
wlr-output-management clients use; `handle-output-configured!' runs on
success and `handle-output-configure-failed!' receives NAME and a reason
string on rejection. Because this is the user's own policy speaking it
does not consult `output-configuration-allowed?', which gates external
clients only. Under the nested winit backend mode changes are refused
(the host window fixes the size). Malformed settings raise an error;
returns #t once queued, #f without a compositor."
  (let ((alist
         (filter (lambda (entry) (not (eq? (cdr entry) 'unset)))
                 (list (cons 'mode mode)
                       (cons 'position position)
                       (cons 'scale scale)
                       (cons 'transform transform)
                       (cons 'enabled enabled)
                       (cons 'adaptive-sync adaptive-sync)))))
    (let* ((mod (resolve-module '(guile-user) #:ensure #f))
           (var (and mod (module-variable mod 'wm-configure-output!))))
      (and var ((variable-ref var) name alist) #t))))

(define (output-configuration-allowed?)
  "Policy predicate consulted before an external wlr-output-management
client (wlr-randr, kanshi, shikane, wdisplays) may test or apply an
output configuration. The default accepts everything; redefine it in
your init file to return #f (or gate on your own state) to refuse
external changes. `configure-output!' does not consult it."
  #t)

(define (handle-output-configured!)
  "Rust: the output layout was just changed -- by an external
wlr-output-management client or by `configure-output!'. The default
does nothing; redefine it to re-tile, persist or log. The head model
has already been reflowed through `handle-heads-change!' by the time
this runs."
  *unspecified*)

(define (handle-output-configure-failed! name reason)
  "Rust: a `configure-output!' request for the head NAME was rejected
for REASON (a string). The default logs it through `wm-log'."
  (rust-call 'wm-log
             (format #f "configure-output!: ~a: ~a" name reason)))

;; ---------------------------------------------------------------------
;; Keyboard layout groups. minde builds one keymap from the standard
;; XKB_DEFAULT_* variables. A comma-separated list of layouts, such as
;; XKB_DEFAULT_LAYOUT="us,de", yields several groups that these
;; wrappers (over `wm-keyboard-layouts' / `wm-set-keyboard-layout!')
;; switch between -- an in-compositor alternative to grp:* toggle
;; options, so the switch can live under the prefix key.
;; ---------------------------------------------------------------------

(define (keyboard-layouts)
  "The XKB layout groups of the seat keyboard, in keymap order, as a list
of alists ((name . \"English (US)\") (active . #t)). Groups come from
XKB_DEFAULT_LAYOUT (comma separated) at startup; a single-layout keymap
gives one entry. Returns '() without a compositor."
  (let ((layouts (rust-call 'wm-keyboard-layouts)))
    (if (list? layouts) layouts '())))

(define (%set-keyboard-layout-primitive spec)
  (let* ((mod (resolve-module '(guile-user) #:ensure #f))
         (var (and mod (module-variable mod 'wm-set-keyboard-layout!))))
    (and var ((variable-ref var) spec) #t)))

(define (set-keyboard-layout! spec)
  "Activate a keyboard layout group. SPEC is a zero-based index into
`keyboard-layouts', the symbol `next' or `prev' (cycling with wrap-around),
or a string matched case-insensitively as a prefix of a group's name
(\"English\" selects \"English (US)\"). The switch is queued to the
compositor, which tells the focused client through the normal modifiers
event; `handle-keyboard-layout-changed!' then runs with the new name.
Returns #t once queued, #f without a compositor or for an unknown name;
a malformed SPEC raises."
  (cond
   ((string? spec)
    (let* ((needle (string-downcase spec))
           (index (list-index
                   (lambda (layout)
                     (string-prefix? needle
                                     (string-downcase
                                      (assq-ref layout 'name))))
                   (keyboard-layouts))))
      (and index (%set-keyboard-layout-primitive index))))
   ((or (memq spec '(next prev)) (and (integer? spec) (>= spec 0)))
    (%set-keyboard-layout-primitive spec))
   (else (error "set-keyboard-layout!: expected index, next, prev or name"
                spec))))

(define (handle-keyboard-layout-changed! name)
  "Rust: the active keyboard layout group is now NAME (a string such as
\"English (US)\") -- after `set-keyboard-layout!' or an XKB toggle option
like grp:win_space_toggle. The default shows NAME through the message
overlay; redefine it to update a bar or stay silent."
  (rust-call 'wm-message name))

;; ---------------------------------------------------------------------
;; Session lock/unlock (Rust-facing, interface contract with the
;; compositor's ext-session-lock-v1 support): defined here, alongside
;; the other 0/N-arg Rust event entry points above, rather than in
;; (minde session), because Rust looks these up by plain top-level
;; name (see rust-call's comment) and this is the established home for
;; that. They run user-extensible named hooks; (minde
;; session)'s suspend! adds a one-shot 'session-lock hook of its own to
;; know when the lock surface is actually up before it suspends.
;; ---------------------------------------------------------------------

(define (wm-on-session-lock)
  "Rust: the session-lock surface is up and the session is now locked.
Cancels frame selection and runs the 'session-lock hook."
  (cancel-frame-selection!)
  (run-event-hook! 'session-lock))

(define (wm-on-session-unlock)
  "Rust: the session-lock surface was torn down; the session is
unlocked again. Runs the 'session-unlock hook."
  (run-event-hook! 'session-unlock))

(define (wm-on-session-lock-lost)
  "Rust: the lock owner disappeared; the session remains locked.
Runs the 'session-lock-lost hook once for that owner."
  (run-event-hook! 'session-lock-lost))

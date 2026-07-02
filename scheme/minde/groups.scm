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
  #:export (switch-to-group!
            gother!
            add-placement-rule!
            clear-placement-rules!
            place-existing-windows!
            status-line
            gnext!
            gprev!
            gnew!
            gnew-auto!
            gnew-float!
            gnewbg!
            gnewbg-float!
            gnext-with-window!
            gprev-with-window!
            gmerge!
            gkill-other!
            gmove-marked-to!
            kill-windows-current-group!
            kill-windows-other!
            groups-echo-string
            wm-on-window-moved
            grename!
            gkill!
            move-window-to-next-group!
            gmove-and-follow!
            next-urgent!
            wm-on-heads-changed
            set-head-mode!
            wm-on-window-map
            wm-on-window-unmap
            wm-on-output-geometry
            current-group-name
            group-names
            group-has-window?))

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
      (run-hook!* 'focus-group (group-name target))
      (echo (group-list-string)))))

;; The group that was current before the last switch (StumpWM gother).
(define %last-group #f)

(define (gother!)
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

(define (gnext!)
  "Switches to the next group in %groups, wrapping around."
  (let ((idx (current-group-index)) (n (length %groups)))
    (when (and idx (> n 1))
      (switch-to-group! (modulo (+ idx 1) n)))))

(define (gprev!)
  "Switches to the previous group in %groups, wrapping around."
  (let ((idx (current-group-index)) (n (length %groups)))
    (when (and idx (> n 1))
      (switch-to-group! (modulo (- idx 1) n)))))

;; ---------------------------------------------------------------------
;; Creating groups
;; ---------------------------------------------------------------------

(define (gnewbg! name)
  "Creates a new empty group named NAME (a string), appended to the end of
%groups, sized to the last known output geometry. Does not switch to it
(StumpWM gnewbg)."
  (let* ((size (current-output-size))
         (g (make-empty-group name (car size) (cadr size))))
    (set! %groups (append %groups (list g)))
    g))

(define (gnew! name)
  "Creates a new empty group named NAME and switches to it (StumpWM
gnew)."
  (let ((g (gnewbg! name)))
    (switch-to-group! (group-name g))
    g))

;; Roman numerals for auto-generated group names (I, II, III already
;; taken by the defaults; gnew-auto! continues IV, V, ...). Falls back to
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

(define (gnew-auto!)
  "Creates a new group with an auto-generated roman-numeral name (\" IV \",
\" V \", ... continuing the default groups' naming) and switches to it."
  (gnew! (string-append " " (integer->roman (+ 1 (length %groups))) " ")))

(define (gnewbg-float! name)
  "Creates a new float group in the background (StumpWM gnewbg-float):
every window mapped while it is current floats automatically. Does not
switch to it."
  (let ((g (gnewbg! name)))
    (set-group-float?! g #t)
    g))

(define (gnew-float! name)
  "Creates a new float group and switches to it (StumpWM gnew-float)."
  (let ((g (gnewbg-float! name)))
    (switch-to-group! (group-name g))
    g))

(define (grename! name)
  "Renames the current group (StumpWM grename). Group names are stored
padded (\" I \"); NAME is padded the same way."
  (unless (string-null? name)
    (set-group-name! (current-group) (string-append " " (string-trim-both name) " "))
    (echo (group-list-string))))

(define (gkill!)
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

(define (gnext-with-window!)
  "Moves the current window to the next group and follows it (StumpWM
gnext-with-window)."
  (gshift-with-window! 1))

(define (gprev-with-window!)
  "Moves the current window to the previous group and follows it (StumpWM
gprev-with-window)."
  (gshift-with-window! -1))

;; Adopts every window of G into the current group and deletes G. The
;; shared core of gmerge! and gkill-other!; callers sync and echo.
(define (merge-group-into-current! g)
  (when (and (not (eq? g (current-group))) (memq g %groups))
    (for-each
     (lambda (id) (move-window-between-groups! id g (current-group)))
     (group-window-ids g))
    (set! %groups (delq g %groups))
    (when (eq? %last-group g) (set! %last-group #f))))

(define (gmerge! name-or-index)
  "Merges the group named NAME-OR-INDEX into the current one: all its
windows (floats stay floating) move here and the emptied group is
deleted (StumpWM gmerge)."
  (let ((g (resolve-target name-or-index)))
    (cond
     ((not g) (echo "no such group"))
     ((eq? g (current-group)) (echo "already the current group"))
     (else
      (merge-group-into-current! g)
      (sync-frames!)
      (echo (group-list-string))))))

(define (gkill-other!)
  "Deletes every group except the current one, merging all their windows
into it (StumpWM gkill-other)."
  (for-each merge-group-into-current! (list-copy %groups))
  (sync-frames!)
  (echo (group-list-string)))

(define (gmove-marked-to! name-or-index)
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
    (ensure-unique-window-number! id (group-all-trees to))))

;; ---------------------------------------------------------------------
;; Placement rules (StumpWM define-frame-preference): route a newly
;; mapped window to a specific group and/or frame by matching its app-id
;; or title.
;; ---------------------------------------------------------------------

;; Each rule: (matcher group-name frame-index follow?). MATCHER is a
;; string matched as a substring against the window's app-id, then its
;; title. GROUP-NAME is compared trimmed (" II " and "II" both work);
;; #f means "the current group". FRAME-INDEX is 0-based in tree order,
;; clamped to the existing leaves. First matching rule wins.
(define %placement-rules '())

(define* (add-placement-rule! matcher #:key (group #f) (frame 0) (follow? #f))
  (set! %placement-rules
        (append %placement-rules (list (list matcher group frame follow?)))))

(define (clear-placement-rules!)
  (set! %placement-rules '()))

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
    (run-hook!* 'new-window id title app-id)
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
           (when rule
             (let* ((g (or (find-group-loose (cadr rule)) (current-group)))
                    (active? (eq? g (current-group)))
                    (tree (if active? (current-tree) (group-tree g)))
                    (leaves (frame-leaves tree))
                    (leaf (list-ref leaves (min (max 0 (caddr rule))
                                                (- (length leaves) 1)))))
               (unless (member id (frame-window-ids leaf))
                 (set! moved (+ moved 1))
                 (find (lambda (gg)
                         (any (lambda (t) (remove-window-from-tree-in! t id))
                              (group-all-trees gg)))
                       %groups)
                 (ensure-unique-window-number! id (group-all-trees g))
                 (frame-add-window! leaf id)
                 (unless active? (hide-window! id))))))))
     (all-window-ids))
    (sync-frames!)
    (echo (format #f "placed ~a window(s)" moved))))

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

(define (gmove-and-follow!)
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

(define (wm-on-window-map id title app-id)
  (let ((rule (find (lambda (r) (rule-matches? r title app-id))
                    %placement-rules)))
    (if rule
        (place-by-rule! rule id title app-id)
        (handle-window-map! id title app-id))))

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

(define (wm-on-window-unmap id)
  "Removes ID from whichever group's tree currently holds it -- the active
group first (which re-syncs), then every hidden group (which doesn't need
a sync since nothing hidden is on-screen)."
  (clear-fullscreen-if-window! id)
  (clear-urgent! id)
  (clear-ontop! id)
  (clear-sticky! id)
  (unmark-window! id)
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
  (run-hook!* 'destroy-window id)
  #t)

(define (wm-on-window-moved id x y w h)
  "Rust reports the final geometry of a super+drag move/resize."
  (handle-window-moved! id x y w h))

(define (wm-on-output-geometry x y width height)
  "Single-head compatibility path (old binaries / winit-era configs)."
  (when (and (> width 0) (> height 0))
    (heads-changed! (list (list 0 x y width height)) %groups)))

(define (wm-on-heads-changed heads)
  "Multi-head backends report the full usable-rect list here:
((id x y w h) ...)."
  (heads-changed! heads %groups))

(define (set-head-mode! mode)
  "'per-head (a frame tree per monitor, StumpWM style) or 'span (one
tree over the union of all monitors)."
  (set-heads-mode! mode %groups))

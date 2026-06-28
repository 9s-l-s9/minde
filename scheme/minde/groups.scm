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
            status-line
            gnext!
            gprev!
            gnew!
            gnew-auto!
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
  (append-map frame-window-ids
              (append-map frame-leaves (group-all-trees g))))

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

(define (gnew! name)
  "Creates a new empty group named NAME (a string), appended to the end of
%groups, sized to the last known output geometry. Does not switch to it."
  (let* ((size (current-output-size))
         (g (make-empty-group name (car size) (cadr size))))
    (set! %groups (append %groups (list g)))
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
\" V \", ... continuing the default groups' naming), appended to
%groups. Does not switch to it."
  (gnew! (string-append " " (integer->roman (+ 1 (length %groups))) " ")))

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
           (find (lambda (t) (remove-window-from-tree-in! t id))
                 (group-all-trees g))
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
      (let ((id (current-frame-window)))
        (when id
          (let ((next (list-ref %groups (modulo (+ idx 1) n))))
            (find (lambda (t) (remove-window-from-tree-in! t id))
                  (group-all-trees (current-group)))
            (hide-window! id)
            (ensure-unique-window-number! id (group-all-trees next))
            (frame-add-window! (group-current-frame next) id)
            (sync-frames!)))))))

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

;; ---------------------------------------------------------------------
;; Status line for external bars (eww etc.): written to
;; $XDG_RUNTIME_DIR/minde-status whenever it changes, via the
;; frames.scm sync hook. Consume with `tail -F` (eww deflisten) or poll
;; `minde-cmd '(status-line)'`.
;; ---------------------------------------------------------------------

(define (status-line)
  (let ((id (current-frame-window)))
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
        (id (current-frame-window)))
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
  (unless (remove-window-from-active-tree! id)
    (find (lambda (g)
            (any (lambda (t) (remove-window-from-tree-in! t id))
                 (group-all-trees g)))
          %groups))
  (forget-window-title! id)
  (forget-window-number! id)
  (run-hook!* 'destroy-window id)
  #t)

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

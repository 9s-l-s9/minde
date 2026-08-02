;;; frames.scm -- StumpWM-style manual frame tiling.
;;;
;;; Owns all layout policy: a binary frame tree, a "current frame" pointer,
;;; and the sync step that pushes computed geometries down to Rust via
;;; `wm-place-window` / `wm-focus-window`.
;;;
;;; Important design constraint (see tests/frames-test.scm): nothing at
;;; module load time calls any `wm-*` Rust subr. Those are only ever called
;;; from `sync-frames!`, `handle-window-map!`, etc. -- i.e. in response to an
;;; event, never as a side effect of loading this file. That's what lets the
;;; unit test stub the `wm-*` procedures *before* loading this module and
;;! still have everything work.

(define-module (minde compositor frames)
  #:use-module (srfi srfi-1)
  #:use-module (srfi srfi-9)
  #:use-module (srfi srfi-11)
  #:use-module (ice-9 regex)
  #:use-module (minde foundation geometry)
  #:use-module (minde compositor model)
  #:use-module (minde hooks)
  #:re-export (group-name group-floats group-float?
               frame-window-ids frame-current-window)
  #:export (split-frame-horizontal!
            split-frame-vertical!
            remove-split!
            focus-next-frame!
            focus-previous-frame!
            focus-next-window!
            focus-previous-window!
            focus-next-window-in-frame!
            focus-previous-window-in-frame!
            pull-hidden-next!
            pull-hidden-previous!
            pull-window-from-other-frame!
            other-window!
            other-frame!
            move-focus!
            move-window!
            exchange-windows!
            collapse-to-one-frame!
            clear-current-frame!
            hsplit-equally!
            vsplit-equally!
            window-number
            assign-window-number!
            forget-window-number!
            ensure-unique-window-number!
            select-window-by-number!
            pull-window-by-number!
            pull-window-by-id!
            renumber-window!
            repack-window-numbers!
            echo-windows-string
            last-message
            fullscreen!
            fullscreen-window
            clear-fullscreen-if-window!
            kill-current-window!
            ratwarp!
            move-pointer-to-corner!
            current-frame-rect
            urgent-windows
            add-urgent-window!
            clear-urgent!
            all-window-ids
            flatten-floats!
            toggle-always-on-top!
            ontop-windows
            clear-ontop!
            rename-window!
            window-send-string
            parse-key-spec
            send-key
            meta
            send-escape
            define-remapped-keys!
            unbind-remapped-keys!
            toggle-remapped-keys!
            remap-target
            ratrelwarp
            ratclick!
            idle-ms
            show-window-properties!
            focus-sibling-frame!
            show-frame-overlays!
            clear-frame-overlays!
            focus-frame-by-index!
            dump-frames
            restore-frames!
            dump-group-frames
            restore-group-frames!
            expose-enter!
            expose-pick!
            unmaximize!
            window-unmaximized?
            set-window-gravity!
            clear-unmaximized!
            set-float-geometry!
            toggle-always-show!
            sticky-windows
            clear-sticky!
            unmark-window!
            window-app-id
            current-frame
            current-frame-window-ids
            float-this!
            float-window!
            unfloat-window!
            window-floating?
            float-geometry
            focused-window-id
            place-floats!
            update-floating-window-geometry!
            remove-float!
            mark-window-toggle!
            marked-windows
            clear-marks!
            pull-marked!
            sync-frames!
            heads
            head-mode
            current-head-id
            heads-changed!
            set-heads-mode!
            focus-head!
            focus-next-head!
            focus-previous-head!
            focus-last-head!
            head-of-window
            group-all-trees
            track-window-map!
            track-float-map!
            track-window-unmap!
            update-output-geometry!
            remove-window-from-active-tree!
            remove-window-from-tree-in!
            current-frame-window
            close-current-window!
            frame-tree-window-count
            echo
            set-gaps!
            resize-frame!
            balance-frames!
            apply-layout-spec!
            dump-layout-spec
            set-sync-hook!
            window-title
            remember-window-title!
            update-window-title!
            forget-window-title!
            window-ids-with-titles
            focus-window-by-id!
            ;; Opaque group operations used by (minde groups). Mutable
            ;; record access lives in the private compositor model module.
            make-empty-group
            current-group
            current-tree
            current-output-size
            activate-group!
            park-group-windows!
            group-window-count
            frame-leaves
            chain-spec
            frame-add-window!
            hide-window!))

;; ---------------------------------------------------------------------
;; Data types
;; ---------------------------------------------------------------------

;; A leaf frame: a rectangle plus the list of window ids assigned to it and
;; which one (if any) is currently shown.
;; Public 3-argument constructor (the last/shown fields start empty; the
;; head table starts with just the tree given, on the current head).
(define (make-group name tree current-frame)
  (make-group-record name tree current-frame #f #f #f #f
                     (make-hash-table) %current-head-id '() #f))

;; ---------------------------------------------------------------------
;; State
;; ---------------------------------------------------------------------

;; ---------------------------------------------------------------------
;; Heads (outputs/monitors). Each head is (id x y w h) -- its usable
;; rect in global coordinates. %raw-heads is what the backend reported;
;; %heads is what the frame layer works with ('span mode collapses the
;; raw list into one synthetic head covering the union).
;; ---------------------------------------------------------------------

(define %raw-heads (list (list 0 0 0 1280 720)))
(define %heads (list (list 0 0 0 1280 720)))
(define %head-mode 'per-head) ; 'per-head | 'span
(define %current-head-id 0)
(define %last-head-id 0)

(define (heads)
  "Returns the effective output heads as (id x y width height) lists."
  %heads)
(define (head-mode)
  "Returns the current output layout mode: 'per-head or 'span."
  %head-mode)
(define (current-head-id)
  "Returns the identifier of the currently focused output head."
  %current-head-id)
(define (head-rect hid) (assv hid %heads))

;; The root of the frame tree -- either a <frame> or a <split> whose leaves
;; are (transitively) <frame>s. This is always the ACTIVE group's tree ON
;; THE CURRENT HEAD; other groups' (and other heads') trees live in their
;; <group> records until activated/focused.
(define %frame-tree
  (make-frame 0 0 1280 720 '() #f))

;; The leaf <frame> that currently has input focus, within %frame-tree.
(define %current-frame %frame-tree)

;; The most recently seen usable area (output minus layer-shell exclusive
;; zones, e.g. a docked eww bar), applied to a group's tree whenever it
;; becomes active (so a group created/hidden before a resize doesn't show
;; up with stale geometry the first time it's synced).
(define %last-output-x 0)
(define %last-output-y 0)
(define %last-output-w 1280)
(define %last-output-h 720)

(define (current-output-size)
  "Returns the current head's usable (width height)."
  (list %last-output-w %last-output-h))

;; The group whose tree/current-frame are currently loaded into
;; %frame-tree/%current-frame above. (minde groups) bootstraps its
;; default group list by grabbing this one and renaming it, so nothing
;; here needs to know about groups plural.
(define %active-group (make-group "default" %frame-tree %current-frame))

(define (current-group)
  "Returns the active group record."
  %active-group)

(define (current-tree)
  "Returns the active group's live frame tree on the current head."
  %frame-tree)

;; Creates a fresh, empty, single-frame group of the given size. Building
;; a <group>/<frame> record is pure data construction -- no wm-* calls --
;; so this is safe to use at module load time too.
(define (make-empty-group name w h)
  "Returns a new group named NAME with one empty W by H frame."
  (let ((f (make-frame 0 0 w h '() #f)))
    (make-group name f f)))

;; Writes the live %frame-tree/%current-frame back into %active-group's
;; own fields (which otherwise go stale the moment a split/remove-split
;; replaces the tree root with a new object).
(define (flush-active-group!)
  (set-group-tree! %active-group %frame-tree)
  (set-group-current-frame! %active-group %current-frame))

;; Makes G the active group: flushes the previously-active group's live
;; state into its record, loads G's stored tree/current-frame into
;; %frame-tree/%current-frame, and re-stretches G's tree to the last known
;; output size. Does not sync or touch any wm-* subr -- callers (typically
;; (minde groups)'s switch-to-group!) are expected to park the outgoing
;; group's windows first and call sync-frames! afterwards.
(define (activate-group! g)
  "Loads G into the live frame state without parking or synchronizing windows."
  (unless (eq? g %active-group)
    (flush-active-group!)
    (load-head-into-group! g %current-head-id)
    (set! %frame-tree (group-tree g))
    (set! %current-frame (group-current-frame g))
    (set! %active-group g)
    (resize-subtree! %frame-tree %last-output-x %last-output-y
                     %last-output-w %last-output-h)))

;; ---------------------------------------------------------------------
;; Multi-head plumbing
;; ---------------------------------------------------------------------

;; Every tree of G, across all heads (the live/loaded one first).
(define (group-all-trees g)
  "Returns every frame tree owned by G, with its loaded tree first."
  (cons (if (eq? g %active-group) %frame-tree (group-tree g))
        (hash-map->list (lambda (hid pair) (car pair)) (group-heads g))))

;; (rect . tree) pairs for every head tree of the ACTIVE group, where
;; rect is that head's (x y w h) -- used by sync-frames-now! so gap
;; math sees the right screen bounds per head.
(define (active-head-trees)
  (cons (cons (cdr (or (head-rect %current-head-id)
                       (list 0 %last-output-x %last-output-y
                             %last-output-w %last-output-h)))
              %frame-tree)
        (hash-map->list (lambda (hid pair)
                          (cons (cdr (or (head-rect hid)
                                         (list hid 0 0 1280 720)))
                                (car pair)))
                        (group-heads %active-group))))

(define (active-leaves)
  (append-map frame-leaves (group-all-trees %active-group)))

(define (set-last-output-from-head!)
  (let ((r (head-rect %current-head-id)))
    (when r
      (set! %last-output-x (cadr r))
      (set! %last-output-y (caddr r))
      (set! %last-output-w (cadddr r))
      (set! %last-output-h (car (cddddr r))))))

;; Makes G's tree/current-frame fields refer to head HID, stashing the
;; previously loaded head's pair in the heads hash. A head no group has
;; touched yet gets a fresh full-rect frame lazily.
(define (load-head-into-group! g hid)
  (when (and (head-rect hid) (not (eqv? (group-loaded-head g) hid)))
    (let ((h (group-heads g)))
      ;; Stash even if the old head no longer exists: heads-changed!'s
      ;; adoption pass picks removed heads' windows out of the hash.
      (hash-set! h (group-loaded-head g)
                 (cons (group-tree g) (group-current-frame g)))
      (let ((pair (or (hash-ref h hid)
                      (let* ((r (head-rect hid))
                             (f (make-frame (cadr r) (caddr r) (cadddr r)
                                            (car (cddddr r)) '() #f)))
                        (cons f f)))))
        (hash-remove! h hid)
        (set-group-tree! g (car pair))
        (set-group-current-frame! g (cdr pair))
        (set-group-loaded-head! g hid)))))

(define (focus-head! hid)
  "Makes head HID the current one: the active group's tree on that head
becomes the live tree (StumpWM screen focus)."
  (when (and (head-rect hid) (not (eqv? hid %current-head-id)))
    (flush-active-group!)
    (set! %last-head-id %current-head-id)
    (set! %current-head-id hid)
    (load-head-into-group! %active-group hid)
    (set! %frame-tree (group-tree %active-group))
    (set! %current-frame (group-current-frame %active-group))
    (set-last-output-from-head!)
    (sync-frames!)))

(define (sorted-head-ids)
  (map car (sort %heads (lambda (a b)
                          (or (< (cadr a) (cadr b))
                              (and (= (cadr a) (cadr b))
                                   (< (caddr a) (caddr b))))))))

(define (shift-head! dir)
  (let* ((ids (sorted-head-ids))
         (n (length ids)))
    (if (< n 2)
        (echo "only one head")
        (let ((idx (or (list-index (lambda (i) (eqv? i %current-head-id)) ids)
                       0)))
          (focus-head! (list-ref ids (modulo (+ idx dir) n)))))))

(define (focus-next-head!) (shift-head! 1))
(define (focus-previous-head!)
  "Focuses the previous output head in display order."
  (shift-head! -1))

(define (focus-last-head!)
  "Toggles to the previously focused head (StumpWM sother)."
  (if (eqv? %last-head-id %current-head-id)
      (echo "only one head")
      (focus-head! %last-head-id)))

;; The head beyond the current one in DIR, best perpendicular overlap
;; wins -- lets directional focus cross monitor bezels.
(define (head-in-direction dir)
  (let ((head (directional-neighbor
               (head-rect %current-head-id) %heads dir
               #:rectangle (lambda (item) (cdr item))
               #:same? (lambda (a b) (eqv? (car a) (car b)))
               #:adjacent? #f)))
    (and head (car head))))

;; The union bounding box of RAW heads, as a single head reusing the
;; first raw head's id (so that head's trees survive a mode switch).
(define (effective-heads raw)
  (if (or (eq? %head-mode 'per-head) (null? raw) (null? (cdr raw)))
      raw
      (let ((x1 (apply min (map cadr raw)))
            (y1 (apply min (map caddr raw)))
            (x2 (apply max (map (lambda (r) (+ (cadr r) (cadddr r))) raw)))
            (y2 (apply max (map (lambda (r) (+ (caddr r) (car (cddddr r)))) raw))))
        (list (list (caar raw) x1 y1 (- x2 x1) (- y2 y1))))))

(define (heads-changed! raw groups)
  "The backend's head list changed (hotplug, resize, exclusive zones).
RAW is ((id x y w h) ...) usable rects; GROUPS is every group (the
active one may be omitted -- it is included automatically). New heads
get lazy empty trees; removed heads' windows are adopted into each
group's surviving current head; survivors are resized."
  (unless (null? raw)
    (set! %raw-heads raw)
    (apply-effective-heads! (effective-heads raw) groups)))

(define (set-heads-mode! mode groups)
  "Switches between 'per-head (a frame tree per monitor, StumpWM style)
and 'span (one tree over the union of all monitors)."
  (set! %head-mode mode)
  (apply-effective-heads! (effective-heads %raw-heads) groups))

(define (apply-effective-heads! new groups)
  (flush-active-group!)
  (let* ((old-ids (map car %heads))
         (new-ids (map car new))
         (removed (filter (lambda (i) (not (memv i new-ids))) old-ids))
         (all-groups (if (memq %active-group groups)
                         groups
                         (cons %active-group groups))))
    (set! %heads new)
    (clamp-floats-to-heads! new)
    (unless (memv %current-head-id new-ids)
      (set! %current-head-id (car new-ids)))
    (unless (memv %last-head-id new-ids)
      (set! %last-head-id %current-head-id))
    (for-each
     (lambda (g)
       ;; Reconcile the loaded pair onto a surviving head first, then
       ;; adopt windows stranded on removed heads into it.
       (load-head-into-group! g %current-head-id)
       (for-each
        (lambda (hid)
          (let ((pair (hash-ref (group-heads g) hid)))
            (when pair
              (hash-remove! (group-heads g) hid)
              (let ((ids (tree-window-ids (car pair)))
                    (target (group-current-frame g)))
                (for-each
                 (lambda (id)
                   (set-frame-window-ids!
                    target (append (frame-window-ids target) (list id))))
                 ids)
                (when (and (pair? ids) (not (frame-current-window target)))
                  (set-frame-current-window! target (car ids)))))))
        removed)
       ;; Resize every surviving tree to its head's (new) rect.
       (hash-for-each
        (lambda (hid pair)
          (let ((r (head-rect hid)))
            (when r
              (apply resize-subtree! (car pair) (cdr r)))))
        (group-heads g))
       (let ((r (head-rect (group-loaded-head g))))
         (when r
           (apply resize-subtree! (group-tree g) (cdr r)))))
     all-groups)
    ;; The active group's record may have been reloaded above; refresh
    ;; the live globals and the current-head compat vars, then sync.
    (set! %frame-tree (group-tree %active-group))
    (set! %current-frame (group-current-frame %active-group))
    (set-last-output-from-head!)
    (sync-frames!)))

;; Moves every window tracked by G (active or not) off-screen, without
;; touching focus. Used when a group is about to be hidden.
(define (park-group-windows! g)
  "Moves every tiled and floating window in G off screen."
  (for-each
   (lambda (frame)
     (for-each
      (lambda (id) (wm-place-window id %offscreen-x %offscreen-y (frame-w frame) (frame-h frame)))
      (frame-window-ids frame)))
   (append-map frame-leaves (group-all-trees g)))
  ;; Park the group's floats too (keeping their size; wm-place-float so
  ;; they don't pick up tiled states while hidden).
  (for-each
   (lambda (id)
     (let ((r (hash-ref %floating id)))
       (rust-call 'wm-place-float id %offscreen-x %offscreen-y
                  (if r (caddr r) 100) (if r (cadddr r) 100))))
   (group-floats g)))

;; Total window count tracked by G (active or not), across all heads.
(define (group-window-count g)
  "Returns the number of tiled and floating windows owned by G."
  (+ (length (group-floats g))
     (apply + (map (lambda (f) (length (frame-window-ids f)))
                   (append-map frame-leaves (group-all-trees g))))))

;; Parks a single window off-screen -- used when a window is moved out of
;; the active group's tree into a hidden group (so it doesn't linger
;; on-screen with stale geometry; see move-window-to-next-group! in
;; (minde groups)).
(define (hide-window! id)
  "Moves window ID off screen without changing frame bookkeeping or focus."
  (wm-place-window id %offscreen-x %offscreen-y (frame-w %current-frame) (frame-h %current-frame)))

;; ---------------------------------------------------------------------
;; Calling out to Rust (or, in tests, stubs)
;; ---------------------------------------------------------------------
;;
;; wm-place-window / wm-focus-window / wm-close-window are Rust subrs
;; defined at the top level of whichever module loads us -- (guile-user)
;; in both the real compositor (scheme/init.scm, loaded via
;; scm_c_primitive_load) and tests/frames-test.scm. A module created with
;; define-module doesn't automatically see another module's top-level
;; bindings, and #:use-module/#:select requires the binding to already
;; exist at *compile time* of this file, which is too early (the caller
;; hasn't defined its stubs/Rust hasn't registered its subrs yet when this
;; module itself is compiled). So look them up dynamically by name at call
;; time instead, exactly like the Rust side's own `guile::lookup` does for
;; `wm-handle-key` -- a missing definition is simply a no-op.
(define %reported-missing-rust-calls (make-hash-table))

(define (rust-call name . args)
  (let* ((mod (resolve-module '(guile-user) #:ensure #f))
         (var (and mod (module-variable mod name))))
    (if var
        (apply (variable-ref var) args)
        (begin
          ;; Unit tests intentionally omit capabilities irrelevant to the
          ;; behavior under test. Report each missing name once so that signal
          ;; remains visible without burying real failures in repeated noise.
          (unless (hash-ref %reported-missing-rust-calls name)
            (hash-set! %reported-missing-rust-calls name #t)
            (format #t "minde: ~a unbound, ignoring call~%" name))
          #f))))

(define (wm-place-window id x y w h) (rust-call 'wm-place-window id x y w h))

;; The most recent echoes, newest first (StumpWM lastmsg).
(define %message-history '())

(define (last-message)
  "Returns the most recent compositor message, or #f if none was emitted."
  (and (pair? %message-history) (car %message-history)))

(define (echo text)
  "Shows TEXT in the compositor's message overlay (StumpWM's message
window), falling back to the log when running against a binary without
wm-message (or under the test stubs)."
  (set! %message-history (cons text (take %message-history
                                           (min 19 (length %message-history)))))
  (run-event-hook! 'message text)
  (let ((mod (resolve-module '(guile-user) #:ensure #f)))
    (if (and mod (module-variable mod 'wm-message))
        (rust-call 'wm-message text)
        (rust-call 'wm-log text))))
(define (wm-focus-window id) (rust-call 'wm-focus-window id))
(define (wm-close-window id) (rust-call 'wm-close-window id))
(define (wm-clear-focus) (rust-call 'wm-clear-focus))
(define (wm-focus-rect x y w h) (rust-call 'wm-focus-rect x y w h))

;; ---------------------------------------------------------------------
;; Tree walking helpers
;; ---------------------------------------------------------------------

;; Collects all leaf frames, left/top-to-right/bottom-most first.
(define (frame-leaves node)
  "Returns NODE's leaf frames in top-to-bottom, left-to-right tree order."
  (if (frame-node? node)
      (list node)
      (append (frame-leaves (split-child-a node))
              (frame-leaves (split-child-b node)))))

;; Finds the parent <split> of LEAF within NODE, or #f if LEAF is NODE
;; itself or not found. Returns (values parent side) where side is 'a or
;; 'b, or (values #f #f).
(define (find-parent node leaf)
  (cond
   ((frame-node? node) (values #f #f))
   ((eq? (split-child-a node) leaf) (values node 'a))
   ((eq? (split-child-b node) leaf) (values node 'b))
   (else
    (call-with-values (lambda () (find-parent (split-child-a node) leaf))
      (lambda (p s)
        (if p
            (values p s)
            (find-parent (split-child-b node) leaf)))))))

(define (frame-tree-window-count)
  "Total number of windows tracked across all frames (for tests)."
  (apply + (map (lambda (f) (length (frame-window-ids f))) (frame-leaves %frame-tree))))

;; ---------------------------------------------------------------------
;; Window <-> frame bookkeeping
;; ---------------------------------------------------------------------

;; id -> (title . app-id), remembered from track-window-map! for the
;; windowlist and `info`-style echoes. Survives moves between
;; frames/groups; dropped on unmap.
(define %window-titles (make-hash-table))

(define (window-title id)
  "Returns the remembered display title for window ID."
  (let ((e (hash-ref %window-titles id)))
    (if e (car e) (format #f "window ~a" id))))

(define (window-app-id id)
  "The window's app-id (X11 class), or #f if unknown."
  (let ((e (hash-ref %window-titles id)))
    (and e (not (string-null? (cdr e))) (cdr e))))

(define (forget-window-title! id)
  "Removes window ID's remembered client and user-supplied titles."
  (hash-remove! %window-titles id)
  (hash-remove! %renamed-windows id))

(define (remember-window-title! id title app-id)
  "Stores TITLE and APP-ID metadata for window ID."
  (let ((class (if (string? app-id) app-id "")))
    (hash-set! %window-titles id
               (cons (if (and (string? title) (not (string-null? title)))
                         title
                         class)
                     class))))

;; Windows renamed via rename-window!: their title override sticks over
;; client-driven title changes (StumpWM title behavior).
(define %renamed-windows (make-hash-table))

(define (update-window-title! id title app-id)
  "Client-driven title/app-id change after map (handle-window-title-change!):
refreshes the bookkeeping, keeping a rename-window! title override."
  (let ((kept (if (and (hash-ref %renamed-windows id)
                       (hash-ref %window-titles id))
                  (car (hash-ref %window-titles id))
                  title)))
    (remember-window-title! id kept app-id)))

(define (window-ids-with-titles)
  "All windows of the active group, as (id . title) pairs in frame order."
  (map (lambda (id) (cons id (window-title id))) (all-window-ids)))

;; ---------------------------------------------------------------------
;; Window numbers (StumpWM: every window gets the smallest free number
;; in its group; Print 0-9 select by number).
;; ---------------------------------------------------------------------

;; id -> number. Global map, but numbers are only kept unique within a
;; group (a window lives in exactly one group's tree).
(define %window-numbers (make-hash-table))

(define (window-number id)
  "Returns window ID's group-local selection number, or #f."
  (hash-ref %window-numbers id))

;; TREE may also be a LIST of trees (e.g. group-all-trees output), so
;; number bookkeeping can span a group's heads.
(define (tree-window-ids tree)
  (if (or (null? tree) (pair? tree))
      (append-map tree-window-ids tree)
      (append-map frame-window-ids (frame-leaves tree))))

(define (used-numbers-in tree except-id)
  (filter-map (lambda (id)
                (and (not (equal? id except-id))
                     (hash-ref %window-numbers id)))
              (tree-window-ids tree)))

(define (smallest-free used)
  (let loop ((n 0)) (if (memv n used) (loop (+ n 1)) n)))

(define (assign-window-number! id tree)
  "Gives ID the smallest number not used by another window of TREE."
  (hash-set! %window-numbers id (smallest-free (used-numbers-in tree id))))

(define (forget-window-number! id)
  "Removes the group-local selection number assigned to window ID."
  (hash-remove! %window-numbers id))

(define (ensure-unique-window-number! id tree)
  "Keeps ID's number if free within TREE (its new group), else assigns a
fresh one -- for windows moved between groups."
  (let ((n (hash-ref %window-numbers id)))
    (when (or (not n) (memv n (used-numbers-in tree id)))
      (assign-window-number! id tree))))

(define (window-id-by-number n)
  (find (lambda (id) (eqv? n (hash-ref %window-numbers id)))
        (all-window-ids)))

(define (select-window-by-number! n)
  "Jumps to the active group's window number N, wherever it lives."
  (let ((id (window-id-by-number n)))
    (if id
        (focus-window-by-id! id)
        (echo (format #f "no window ~a" n)))))

(define (pull-window-by-id! id)
  "Pulls window ID (of the active group) into the current frame."
  (cond
   ;; Pulling a float = unfloat it into the current frame.
   ((window-floating? id) (unfloat-window! id))
   (else
    (let ((f (frame-of-window id)))
      (when f
        (unless (eq? f %current-frame)
          (set-frame-window-ids! f (delete id (frame-window-ids f)))
          (when (equal? (frame-current-window f) id)
            (set-frame-current-window!
             f (if (null? (frame-window-ids f)) #f (car (frame-window-ids f))))))
        (frame-add-window! %current-frame id)
        (sync-frames!))))))

(define (pull-window-by-number! n)
  "Pulls the active group's window number N into the current frame."
  (let ((id (window-id-by-number n)))
    (if id
        (pull-window-by-id! id)
        (echo (format #f "no window ~a" n)))))

(define (renumber-window! n)
  "Gives the current window the number N; if another window of the group
holds N, the two swap (StumpWM renumber)."
  (let ((id (current-frame-window)))
    (when id
      (let ((holder (window-id-by-number n))
            (old (hash-ref %window-numbers id)))
        (when (and holder (not (equal? holder id)))
          (hash-set! %window-numbers holder old))
        (hash-set! %window-numbers id n)))))

(define (repack-window-numbers!)
  "Renumbers the active group's windows 0.. in current-number order."
  (let ((ids (sort (all-window-ids)
                   (lambda (a b) (< (or (window-number a) 999)
                                    (or (window-number b) 999))))))
    (let loop ((ids ids) (n 0))
      (unless (null? ids)
        (hash-set! %window-numbers (car ids) n)
        (loop (cdr ids) (+ n 1))))))

(define (echo-windows-string)
  "StumpWM's `windows` echo: \"0*Term  1-Editor  2 zen\" -- * marks the
current window, - the previous one (other-window!'s target)."
  (let ((cur (focused-window-id))
        (last (group-last-window %active-group))
        (ids (sort (all-window-ids)
                   (lambda (a b) (< (or (window-number a) 999)
                                    (or (window-number b) 999))))))
    (if (null? ids)
        "no windows"
        (string-join
         (map (lambda (id)
                (format #f "~a~a~a"
                        (or (window-number id) "?")
                        (cond ((equal? id cur) "*")
                              ((equal? id last) "-")
                              (else " "))
                        (window-title id)))
              ids)
         "  "))))

(define (focus-window-by-id! id)
  "Jumps to window ID wherever it lives in the active group -- switching
heads if needed: its frame becomes current and it is raised. A floating
window just gets float focus (and comes to the top of the float stack)."
  (if (window-floating? id)
      (when (member id (group-floats %active-group))
        (set-group-floats! %active-group
                           (cons id (delete id (group-floats %active-group))))
        (set! %focused-float id)
        (sync-frames!))
      (let ((hid (head-of-window id)))
        (when hid
          (unless (eqv? hid %current-head-id)
            (focus-head! hid))
          (let ((f (find (lambda (fr) (member id (frame-window-ids fr)))
                         (frame-leaves %frame-tree))))
            (when f
              (clear-float-focus!)
              (set! %current-frame f)
              (set-frame-current-window! f id)
              (sync-frames!)))))))

(define (frame-add-window! frame id)
  "Appends window ID to FRAME and makes it FRAME's current window."
  (set-frame-window-ids! frame (append (frame-window-ids frame) (list id)))
  (set-frame-current-window! frame id))

;; Removes ID from every frame of TREE it appears in (there should be at
;; most one). Returns #t if it was found and removed. Generic over any
;; tree so (minde groups) can search hidden groups' trees too.
(define (remove-window-from-tree-in! tree id)
  "Removes ID from TREE and returns whether it was present."
  (let ((found #f))
    (for-each
     (lambda (frame)
       (when (member id (frame-window-ids frame))
         (set! found #t)
         (set-frame-window-ids! frame (delete id (frame-window-ids frame)))
         (when (equal? (frame-current-window frame) id)
           (set-frame-current-window!
            frame
            (if (null? (frame-window-ids frame)) #f (car (frame-window-ids frame)))))))
     (frame-leaves tree))
    found))

;; Removes ID from the active group's trees (any head). Returns #t if
;; found, in which case the active group is re-synced.
(define (remove-window-from-active-tree! id)
  "Removes ID from any active-group head tree, synchronizing on success."
  (let ((found (any (lambda (t) (remove-window-from-tree-in! t id))
                    (group-all-trees %active-group))))
    (when found (sync-frames!))
    found))

(define (current-frame-window)
  "Returns the current frame's selected window identifier, or #f."
  (frame-current-window %current-frame))

;; The live current frame itself -- for callers ((minde groups)'
;; window moves, init.scm's frame-windowlist) that must target it;
;; the active group's group-current-frame field can be stale between
;; flushes.
(define (current-frame)
  "Returns the live current frame record."
  %current-frame)

(define (current-frame-window-ids)
  "Returns the window identifiers assigned to the live current frame."
  (frame-window-ids %current-frame))

;; ---------------------------------------------------------------------
;; Floating windows (StumpWM float-this / unfloat-this / gnew-float).
;; A floated window leaves the frame tree entirely: it keeps arbitrary
;; geometry in %floating, renders above the tiling (place-floats!
;; re-raises after every sync), and can be moved/resized with
;; super+drag (Rust reports the result via handle-window-move!).
;; ---------------------------------------------------------------------

;; id -> (x y w h), the authoritative float geometry.
(define %floating (make-hash-table))

;; The float that currently has keyboard focus, or #f when focus is in
;; the frame tree. Checked against the active group's float list, so a
;; stale id (unmapped, moved to another group) falls back to the frames.
(define %focused-float #f)

(define (window-floating? id)
  "Returns true when window ID has managed floating geometry."
  (and (hash-ref %floating id) #t))

(define (float-geometry id)
  "Returns floating window ID's (x y width height), or #f."
  (hash-ref %floating id))

(define (set-float-geometry! id rect)
  "Overrides a float's remembered (x y w h) -- desktop restore; callers
sync."
  (when (hash-ref %floating id)
    (hash-set! %floating id rect)))

(define (focused-window-id)
  "The window that effectively has focus: the focused float if there is
one in the active group, else the current frame's current window."
  (if (and %focused-float (member %focused-float (group-floats %active-group)))
      %focused-float
      (current-frame-window)))

(define (clear-float-focus!)
  (set! %focused-float #f))

;; Default float rect: centered over the current frame at 2/3 size.
(define (default-float-rect)
  (let* ((r (frame-display-rect %current-frame))
         (w (max 100 (quotient (* 2 (caddr r)) 3)))
         (h (max 80 (quotient (* 2 (cadddr r)) 3))))
    (list (+ (car r) (quotient (- (caddr r) w) 2))
          (+ (cadr r) (quotient (- (cadddr r) h) 2))
          w h)))

(define (add-float! g id rect)
  "Registers ID as a float of G at RECT (bookkeeping + the Rust flag;
callers sync)."
  (hash-set! %floating id rect)
  (set-group-floats! g (cons id (delete id (group-floats g))))
  (rust-call 'wm-set-floating id #t))

(define (remove-float! g id)
  "Drops ID from G's float list and the geometry table (and the Rust
flag). Returns #t if it was floating there. Callers re-add it to a
frame and/or sync as appropriate."
  (and (member id (group-floats g))
       (begin
         (set-group-floats! g (delete id (group-floats g)))
         (hash-remove! %floating id)
         (rust-call 'wm-set-floating id #f)
         (when (equal? %focused-float id) (set! %focused-float #f))
         #t)))

(define* (float-window! id #:optional (rect #f))
  "Takes ID out of the active group's frame trees and floats it."
  (unless (window-floating? id)
    (clear-unmaximized! id) ; floating supersedes the unmaximize rect
    (let ((r (or rect (default-float-rect))))
      (any (lambda (t) (remove-window-from-tree-in! t id))
           (group-all-trees %active-group))
      (add-float! %active-group id r)
      (set! %focused-float id)
      (sync-frames!))))

(define (unfloat-window! id)
  "Puts floating window ID back into the current frame."
  (when (remove-float! %active-group id)
    (frame-add-window! %current-frame id)
    (sync-frames!)))

(define (float-this!)
  "Toggles floating on the focused window (StumpWM float-this /
unfloat-this collapsed into one command)."
  (let ((id (focused-window-id)))
    (cond
     ((not id) (echo "no window"))
     ((window-floating? id)
      (unfloat-window! id)
      (echo (format #f "unfloated ~a" (window-title id))))
     (else
      (float-window! id)
      (echo (format #f "floated ~a" (window-title id)))))))

(define (place-floats!)
  "Places and raises the active group's floats, bottom of the stacking
order first so the head of the floats list ends up on top. Called at
the end of every sync (after the focus step, whose raise would
otherwise put a tiled window above the floats)."
  (for-each
   (lambda (id)
     (let ((r (hash-ref %floating id)))
       (when r
         (rust-call 'wm-place-float id (car r) (cadr r) (caddr r) (cadddr r))
         (rust-call 'wm-raise-window id))))
   (reverse (group-floats %active-group))))

(define (update-floating-window-geometry! id x y w h)
  "Rust reports where a super+drag move/resize ended; keep %floating
authoritative and treat the dragged float as focused/topmost."
  (when (window-floating? id)
    (hash-set! %floating id (list x y w h))
    (when (member id (group-floats %active-group))
      (set-group-floats! %active-group
                         (cons id (delete id (group-floats %active-group))))
      (set! %focused-float id))))

(define (flatten-floats!)
  "Unfloats every float of the active group into the current frame
(StumpWM flatten-floats)."
  (let ((ids (list-copy (group-floats %active-group))))
    (if (null? ids)
        (echo "no floats")
        (begin
          (for-each
           (lambda (id)
             (when (remove-float! %active-group id)
               (frame-add-window! %current-frame id)))
           ids)
          (sync-frames!)
          (echo (format #f "flattened ~a float(s)" (length ids)))))))

;; ---------------------------------------------------------------------
;; Always-on-top (StumpWM toggle-always-on-top): re-raised at the very
;; end of every sync, above the floats.
;; ---------------------------------------------------------------------

(define %ontop-windows '())

(define (ontop-windows)
  "Returns the window identifiers marked always-on-top."
  %ontop-windows)

(define (toggle-always-on-top!)
  "Toggles the focused window's always-on-top flag."
  (let ((id (focused-window-id)))
    (if (not id)
        (echo "no window")
        (begin
          (if (member id %ontop-windows)
              (begin (set! %ontop-windows (delete id %ontop-windows))
                     (echo (format #f "~a: no longer on top" (window-title id))))
              (begin (set! %ontop-windows (append %ontop-windows (list id)))
                     (echo (format #f "~a: always on top" (window-title id)))))
          (sync-frames!)))))

(define (clear-ontop! id)
  "Removes window ID's always-on-top state."
  (set! %ontop-windows (delete id %ontop-windows)))

(define (raise-ontop!)
  (for-each
   (lambda (id)
     (when (member id (all-window-ids))
       (rust-call 'wm-raise-window id)))
   %ontop-windows))

;; ---------------------------------------------------------------------
;; Always-show (StumpWM toggle-always-show): a sticky window follows
;; every group switch -- (minde groups)' switch-to-group! moves the
;; listed windows into the target group before parking the old one.
;; ---------------------------------------------------------------------

(define %sticky-windows '())

(define (sticky-windows)
  "Returns the window identifiers configured to follow group switches."
  %sticky-windows)

(define (toggle-always-show!)
  "Toggles whether the focused window follows every group switch
(StumpWM toggle-always-show)."
  (let ((id (focused-window-id)))
    (if (not id)
        (echo "no window")
        (if (member id %sticky-windows)
            (begin (set! %sticky-windows (delete id %sticky-windows))
                   (echo (format #f "~a: no longer always shown" (window-title id))))
            (begin (set! %sticky-windows (append %sticky-windows (list id)))
                   (echo (format #f "~a: always shown" (window-title id))))))))

(define (clear-sticky! id)
  "Removes window ID's always-show state."
  (set! %sticky-windows (delete id %sticky-windows)))

;; ---------------------------------------------------------------------
;; Small StumpWM leftovers: rename window, send string, ratclick, idle
;; ---------------------------------------------------------------------

(define (rename-window! name)
  "Overrides the focused window's remembered title (StumpWM title)."
  (let ((id (focused-window-id)))
    (when (and id (not (string-null? name)))
      (hash-set! %window-titles id
                 (cons name (let ((e (hash-ref %window-titles id)))
                              (if e (cdr e) ""))))
      (hash-set! %renamed-windows id #t)
      (sync-frames!)   ; status line shows the title
      (echo (format #f "renamed to ~a" name)))))

(define (window-send-string text)
  "Types TEXT into the focused window (StumpWM window-send-string)."
  (rust-call 'wm-send-string text))

(define (ratclick! button)
  "Synthesizes a pointer click at the current position (1=left 2=middle
3=right)."
  (rust-call 'wm-click button))

(define (idle-ms)
  "Milliseconds since the last user input event (0 before any input)."
  (or (rust-call 'wm-idle-ms) 0))

;; ---------------------------------------------------------------------
;; Key synthesis + remapped keys (StumpWM send-raw-key / meta /
;; define-remapped-keys)
;; ---------------------------------------------------------------------

(define (parse-key-spec spec)
  "Splits a binding spec (\"C-M-x\", \"Down\") into (values mods-bitmask
keysym-name), using the same prefixes and bit values as init.scm's
key-spec (C-=ctrl 4, M-=alt 8, S-=shift 1, s-=super 64)."
  (let loop ((s spec) (mods 0))
    (cond
     ((string-prefix? "C-" s) (loop (substring s 2) (logior mods 4)))
     ((string-prefix? "M-" s) (loop (substring s 2) (logior mods 8)))
     ((string-prefix? "S-" s) (loop (substring s 2) (logior mods 1)))
     ((string-prefix? "s-" s) (loop (substring s 2) (logior mods 64)))
     (else (values mods s)))))

(define (send-key spec)
  "Synthesizes one key press/release pair for SPEC (\"C-n\", \"Down\")
into the focused window (StumpWM meta / send-raw-key building block)."
  (let-values (((mods name) (parse-key-spec spec)))
    (rust-call 'wm-send-key mods name)))

(define (meta spec)
  "StumpWM meta: sends SPEC to the focused window."
  (send-key spec))

(define (send-escape)
  "Sends a literal Escape to the focused window (StumpWM send-escape)."
  (send-key "Escape"))

;; Per-application key translation (StumpWM define-remapped-keys): a
;; list of (app-id-regex (from-spec . to-spec) ...). Consulted by
;; init.scm's dispatch for keys that reach the focused client.
(define %remapped-keys '())
(define %remapped-keys-on #t)

(define (define-remapped-keys! specs)
  "Replaces the remap table. SPECS: ((app-id-regex (from . to) ...) ...),
e.g. '((\"zen\" (\"C-n\" . \"Down\") (\"C-p\" . \"Up\")))."
  (set! %remapped-keys specs)
  (echo (format #f "remapped keys: ~a app pattern(s)" (length specs))))

(define (unbind-remapped-keys!)
  "Drops all remap rules (StumpWM unbind-remapped-keys)."
  (set! %remapped-keys '()))

(define (toggle-remapped-keys!)
  "Toggles remapping on/off without forgetting the table; returns the
new state."
  (set! %remapped-keys-on (not %remapped-keys-on))
  %remapped-keys-on)

(define (remap-target spec)
  "The spec SPEC translates to for the focused window's app-id, or #f
(no matching rule, remapping toggled off, or nothing focused)."
  (and %remapped-keys-on
       (pair? %remapped-keys)
       (let ((id (focused-window-id)))
         (and id
              (let ((app (window-app-id id)))
                (and app
                     (any (lambda (entry)
                            (and (string-match (car entry) app)
                                 (assoc-ref (cdr entry) spec)))
                          %remapped-keys)))))))

(define (ratrelwarp dx dy)
  "Warps the pointer by a relative delta (StumpWM ratrelwarp)."
  (rust-call 'wm-warp-pointer-relative dx dy))

(define (show-window-properties!)
  "Echoes the focused window's properties (StumpWM show-window-properties
/ list-window-properties collapsed): id, title, class, number, geometry
source, and flags."
  (let ((id (focused-window-id)))
    (if (not id)
        (echo "no window")
        (echo (format #f "id: ~a~%title: ~a~%class: ~a~%number: ~a~%~a~a~a~a"
                      id (window-title id) (or (window-app-id id) "?")
                      (or (window-number id) "?")
                      (if (window-floating? id)
                          (let ((r (float-geometry id)))
                            (format #f "float: ~ax~a at ~a,~a"
                                    (caddr r) (cadddr r) (car r) (cadr r)))
                          "tiled")
                      (if (member id %marked-windows) ", marked" "")
                      (if (member id %ontop-windows) ", on top" "")
                      (if (member id %sticky-windows) ", always shown" ""))))))

;; Clamps every float rect to the union of the given head rects (called
;; from apply-effective-heads! so unplugging a monitor doesn't strand
;; floats off-screen).
(define (clamp-floats-to-heads! heads)
  (unless (null? heads)
    (let* ((x1 (apply min (map cadr heads)))
           (y1 (apply min (map caddr heads)))
           (x2 (apply max (map (lambda (r) (+ (cadr r) (cadddr r))) heads)))
           (y2 (apply max (map (lambda (r) (+ (caddr r) (car (cddddr r)))) heads))))
      (hash-for-each
       (lambda (id r)
         (let* ((w (min (caddr r) (- x2 x1)))
                (h (min (cadddr r) (- y2 y1)))
                (x (max x1 (min (car r) (- x2 w))))
                (y (max y1 (min (cadr r) (- y2 h)))))
           (hash-set! %floating id (list x y w h))))
       %floating))))

;; ---------------------------------------------------------------------
;; Splitting / removing
;; ---------------------------------------------------------------------

(define (split-current-frame! orientation)
  (let* ((f %current-frame)
         (x (frame-x f)) (y (frame-y f)) (w (frame-w f)) (h (frame-h f))
         (ids (frame-window-ids f))
         (cur (frame-current-window f)))
    (let-values (((ax ay aw ah bx by bw bh)
                  (if (eq? orientation 'horizontal)
                      ;; side by side: split width
                      (let ((half (quotient w 2)))
                        (values x y half h (+ x half) y (- w half) h))
                      ;; stacked: split height
                      (let ((half (quotient h 2)))
                        (values x y w half x (+ y half) w (- h half))))))
      (let ((frame-a (make-frame ax ay aw ah ids cur))
            (frame-b (make-frame bx by bw bh '() #f)))
        (let ((new-split (make-split orientation 1/2 frame-a frame-b)))
          (if (eq? f %frame-tree)
              (set! %frame-tree new-split)
              (let-values (((parent side) (find-parent %frame-tree f)))
                (if (eq? side 'a)
                    (set-split-child-a! parent new-split)
                    (set-split-child-b! parent new-split))))
          (set! %current-frame frame-a))))))

(define (split-frame-horizontal!)
  "Splits the current frame into two side-by-side frames (a vertical
dividing line). The current window (if any) stays in the left frame, which
becomes the new current frame."
  (split-current-frame! 'horizontal)
  (sync-frames!))

(define (split-frame-vertical!)
  "Splits the current frame into two stacked frames (a horizontal dividing
line, StumpWM's \"vsplit\"). The current window (if any) stays in the top
frame, which becomes the new current frame."
  (split-current-frame! 'vertical)
  (sync-frames!))

(define (remove-split!)
  "Removes the current frame; its sibling absorbs the freed space and
becomes current. Any windows the removed frame held are handed to the
surviving sibling (into its first leaf, if the sibling is itself a
subtree) so removing a frame never drops a window. A no-op if the current
frame is the whole tree (nothing to remove)."
  (let-values (((parent side) (find-parent %frame-tree %current-frame)))
    (when parent
      (let* ((removed %current-frame)
             (removed-ids (frame-window-ids removed))
             (sibling (if (eq? side 'a) (split-child-b parent) (split-child-a parent)))
             (rect (subtree-rect parent))
             (gx (car rect)) (gy (cadr rect))
             (gw (caddr rect)) (gh (cadddr rect))
             ;; Sibling (which may itself be a subtree) absorbs the
             ;; parent's whole rectangle, preserving any internal splits'
             ;; proportions; the removed frame's windows land in whichever
             ;; leaf of the sibling ends up first.
             (target-leaf
              (begin
                (if (frame-node? sibling)
                    (begin
                      (set-frame-x! sibling gx) (set-frame-y! sibling gy)
                      (set-frame-w! sibling gw) (set-frame-h! sibling gh))
                    (resize-subtree! sibling gx gy gw gh))
                (car (frame-leaves sibling)))))
        (set-frame-window-ids! target-leaf (append (frame-window-ids target-leaf) removed-ids))
        (unless (frame-current-window target-leaf)
          (set-frame-current-window!
           target-leaf
           (if (null? (frame-window-ids target-leaf)) #f (car (frame-window-ids target-leaf)))))
        (if (eq? parent %frame-tree)
            (set! %frame-tree sibling)
            (let-values (((gp gs) (find-parent %frame-tree parent)))
              (if (eq? gs 'a)
                  (set-split-child-a! gp sibling)
                  (set-split-child-b! gp sibling))))
        (set! %current-frame (car (frame-leaves sibling))))))
  (sync-frames!))

;; The pixel rectangle (x y w h) currently occupied by an entire subtree
;; (leaf or split), i.e. the union of its leaves' rectangles.
(define (subtree-rect node)
  (if (frame-node? node)
      (list (frame-x node) (frame-y node) (frame-w node) (frame-h node))
      (let* ((leaves (frame-leaves node))
             (x0 (apply min (map frame-x leaves)))
             (y0 (apply min (map frame-y leaves)))
             (x1 (apply max (map (lambda (f) (+ (frame-x f) (frame-w f))) leaves)))
             (y1 (apply max (map (lambda (f) (+ (frame-y f) (frame-h f))) leaves))))
        (list x0 y0 (- x1 x0) (- y1 y0)))))

;; Re-stretches a subtree (leaf or split) to occupy exactly the given
;; rectangle, preserving each split's ratio and orientation.
(define (resize-subtree! node x y w h)
  (if (frame-node? node)
      (begin
        (set-frame-x! node x) (set-frame-y! node y)
        (set-frame-w! node w) (set-frame-h! node h))
      (let ((ratio (split-ratio node)))
        (if (eq? (split-orientation node) 'horizontal)
            (let ((aw (inexact->exact (round (* w ratio)))))
              (resize-subtree! (split-child-a node) x y aw h)
              (resize-subtree! (split-child-b node) (+ x aw) y (- w aw) h))
            (let ((ah (inexact->exact (round (* h ratio)))))
              (resize-subtree! (split-child-a node) x y w ah)
              (resize-subtree! (split-child-b node) x (+ y ah) w (- h ah)))))))

;; ---------------------------------------------------------------------
;; Resizing (iresize) and balancing
;; ---------------------------------------------------------------------

;; The nearest ancestor <split> of NODE with the given orientation, or #f.
(define (find-oriented-ancestor node orientation)
  (let loop ((node node))
    (let-values (((parent side) (find-parent %frame-tree node)))
      (cond
       ((not parent) #f)
       ((eq? (split-orientation parent) orientation) parent)
       (else (loop parent))))))

(define (resize-frame! dir step)
  "Moves the divider of the nearest split in direction DIR ('left 'right
'up 'down) by STEP pixels: right/down push the divider right/down,
left/up pull it left/up. A no-op when no split of that orientation
encloses the current frame. Ratio is clamped to [1/10, 9/10]."
  (let* ((orientation (if (memq dir '(left right)) 'horizontal 'vertical))
         (split (find-oriented-ancestor %current-frame orientation)))
    (when split
      (let* ((rect (subtree-rect split))
             (span (if (eq? orientation 'horizontal) (caddr rect) (cadddr rect)))
             (delta (* (/ step span) (if (memq dir '(right down)) 1 -1)))
             (ratio (min 9/10 (max 1/10 (+ (split-ratio split) delta)))))
        (set-split-ratio! split ratio)
        (apply resize-subtree! split rect)
        (sync-frames!)))))

(define (balance-frames!)
  "Resets every split's ratio so all leaf frames get equal shares
(weighted by leaf count on each side), StumpWM's balance-frames."
  (let balance! ((node %frame-tree))
    (unless (frame-node? node)
      (let ((la (length (frame-leaves (split-child-a node))))
            (lb (length (frame-leaves (split-child-b node)))))
        (set-split-ratio! node (/ la (+ la lb)))
        (balance! (split-child-a node))
        (balance! (split-child-b node)))))
  (resize-subtree! %frame-tree %last-output-x %last-output-y
                   %last-output-w %last-output-h)
  (sync-frames!))

;; ---------------------------------------------------------------------
;; Layout specs: a pure-sexp mirror of the tree, for presets and
;; save/restore. Grammar: 'leaf | (hsplit RATIO SPEC SPEC) | (vsplit ...)
;; where hsplit puts its children side by side (like
;; split-frame-horizontal!) and RATIO is child-a's share.
;; ---------------------------------------------------------------------

(define (spec->tree spec)
  (cond
   ((eq? spec 'leaf) (make-frame 0 0 100 100 '() #f))
   ((and (list? spec) (= (length spec) 4) (memq (car spec) '(hsplit vsplit))
         (number? (cadr spec)) (< 0 (cadr spec) 1))
    (make-split (if (eq? (car spec) 'hsplit) 'horizontal 'vertical)
                (cadr spec)
                (spec->tree (caddr spec))
                (spec->tree (cadddr spec))))
   (else (error "bad layout spec" spec))))

(define (apply-layout-spec! spec)
  "Replaces the active group's frame tree with one built from SPEC, sized
to the current usable area. Existing windows are redistributed: the
current window into the first leaf, the rest round-robin over all
leaves. Windows never get lost -- every id ends up in some leaf."
  (let* ((tree (spec->tree spec))
         (leaves (frame-leaves tree))
         (ids (all-window-ids))
         (cur (current-frame-window))
         (ordered (if (and cur (member cur ids)) (cons cur (delete cur ids)) ids)))
    (resize-subtree! tree %last-output-x %last-output-y
                     %last-output-w %last-output-h)
    (let loop ((ids ordered) (i 0))
      (unless (null? ids)
        (frame-add-window! (list-ref leaves (modulo i (length leaves))) (car ids))
        (loop (cdr ids) (+ i 1))))
    ;; Round-robin appends, so a leaf's *first* window should be visible,
    ;; not whichever landed there last.
    (for-each
     (lambda (f)
       (unless (null? (frame-window-ids f))
         (set-frame-current-window! f (car (frame-window-ids f)))))
     leaves)
    (set! %frame-tree tree)
    (set! %current-frame (car leaves))
    (sync-frames!)))

(define (tree-spec tree)
  "TREE as a layout spec; ratios are derived from the actual pixel
rectangles so manual resizes survive a dump/apply round-trip."
  (let node->spec ((node tree))
    (if (frame-node? node)
        'leaf
        (let* ((ra (subtree-rect (split-child-a node)))
               (rr (subtree-rect node))
               (horizontal? (eq? (split-orientation node) 'horizontal))
               (ratio (if horizontal?
                          (/ (caddr ra) (caddr rr))
                          (/ (cadddr ra) (cadddr rr)))))
          (list (if horizontal? 'hsplit 'vsplit)
                ratio
                (node->spec (split-child-a node))
                (node->spec (split-child-b node)))))))

(define (dump-layout-spec)
  "The active group's live tree as a layout spec."
  (tree-spec %frame-tree))

;; ---------------------------------------------------------------------
;; Frame dumps with windows (StumpWM dump-desktop / expose): a layout
;; spec plus which windows sat in which leaf. Shared by expose! and
;; (minde groups)' dump-desktop.
;; ---------------------------------------------------------------------

(define (dump-tree tree cur-frame)
  (let ((leaves (frame-leaves tree)))
    (list (tree-spec tree)
          (map (lambda (f) (list-copy (frame-window-ids f))) leaves)
          (map frame-current-window leaves)
          (or (list-index (lambda (f) (eq? f cur-frame)) leaves) 0))))

(define (dump-frames)
  "Snapshot of the live tree: (spec leaf-window-lists leaf-currents
current-frame-index)."
  (dump-tree %frame-tree %current-frame))

(define (dump-group-frames g)
  "Returns a dump-frames snapshot for group G."
  (if (eq? g %active-group)
      (dump-frames)
      (dump-tree (group-tree g) (group-current-frame g))))

(define (pad-list lst n fill)
  (let ((l (if (> (length lst) n) (list-head lst n) lst)))
    (append l (make-list (- n (length l)) fill))))

;; Rebuilds a tree from a dump-frames snapshot, keeping only window ids
;; in LIVE (stale ids dropped); LIVE windows the dump doesn't mention
;; are appended to the first leaf. Returns (values tree current-leaf).
(define (build-tree-from-dump dump live)
  (let* ((spec (car dump))
         (window-lists (cadr dump))
         (currents (caddr dump))
         (cur-idx (cadddr dump))
         (tree (spec->tree spec))
         (leaves (frame-leaves tree))
         (placed '()))
    (resize-subtree! tree %last-output-x %last-output-y
                     %last-output-w %last-output-h)
    (for-each
     (lambda (leaf ids cur)
       (let ((keep (filter (lambda (id) (member id live)) ids)))
         (set-frame-window-ids! leaf keep)
         (set! placed (append placed keep))
         (set-frame-current-window!
          leaf (if (and cur (member cur keep))
                   cur
                   (if (null? keep) #f (car keep))))))
     leaves
     (pad-list window-lists (length leaves) '())
     (pad-list currents (length leaves) #f))
    (let ((orphans (filter (lambda (id)
                             (and (not (member id placed))
                                  (not (window-floating? id))))
                           live))
          (f (car leaves)))
      (for-each
       (lambda (id)
         (set-frame-window-ids! f (append (frame-window-ids f) (list id)))
         (unless (frame-current-window f)
           (set-frame-current-window! f id)))
       orphans))
    (values tree (list-ref leaves (min cur-idx (- (length leaves) 1))))))

(define (restore-frames! dump)
  "Replaces the active tree with a dump-frames snapshot (stale window
ids dropped, new windows appended to the first leaf) and re-syncs."
  (call-with-values
      (lambda () (build-tree-from-dump dump (tree-window-ids %frame-tree)))
    (lambda (tree cur)
      (set! %frame-tree tree)
      (set! %current-frame cur)
      (sync-frames!))))

(define (restore-group-frames! g dump)
  "restore-frames! for any group: the active one goes through the live
globals; a hidden one just gets its record fields rebuilt (it is resized
and synced on activation)."
  (if (eq? g %active-group)
      (restore-frames! dump)
      (call-with-values
          (lambda () (build-tree-from-dump dump (tree-window-ids (group-tree g))))
        (lambda (tree cur)
          (set-group-tree! g tree)
          (set-group-current-frame! g cur)))))

;; ---------------------------------------------------------------------
;; Frame-number overlays (fselect / expose) + sibling
;; ---------------------------------------------------------------------

(define (show-frame-overlays!)
  "Draws each leaf's index (frame-leaves order, the fselect numbering)
near its top-left corner."
  (rust-call 'wm-clear-overlays)
  (let loop ((leaves (frame-leaves %frame-tree)) (n 0))
    (unless (null? leaves)
      (let ((r (frame-display-rect (car leaves))))
        (rust-call 'wm-add-overlay (+ (car r) 8) (+ (cadr r) 8)
                   (number->string n)))
      (loop (cdr leaves) (+ n 1)))))

(define (clear-frame-overlays!)
  "Removes all numbered frame overlays from the compositor."
  (rust-call 'wm-clear-overlays))

(define (focus-frame-by-index! n)
  "Focuses leaf N in frame-leaves order (fselect target)."
  (let ((leaves (frame-leaves %frame-tree)))
    (if (< n (length leaves))
        (begin
          (clear-float-focus!)
          (set! %current-frame (list-ref leaves n))
          (sync-frames!))
        (echo (format #f "no frame ~a" n)))))

(define (focus-sibling-frame!)
  "Focuses the sibling of the current frame's split (StumpWM sibling)."
  (clear-float-focus!)
  (let-values (((parent side) (find-parent %frame-tree %current-frame)))
    (if (not parent)
        (echo "no sibling")
        (let ((sib (if (eq? side 'a)
                       (split-child-b parent)
                       (split-child-a parent))))
          (set! %current-frame (car (frame-leaves sib)))
          (sync-frames!)))))

;; ---------------------------------------------------------------------
;; Expose (StumpWM expose): temporarily tile every window of the head
;; one-per-frame in a numbered grid, pick one, restore the layout.
;; ---------------------------------------------------------------------

(define %expose-saved #f)

(define (chain-spec orientation k)
  "Returns a layout spec containing K leaves split along ORIENTATION."
  (if (<= k 1)
      'leaf
      (list (if (eq? orientation 'horizontal) 'hsplit 'vsplit)
            (/ 1 k) 'leaf (chain-spec orientation (- k 1)))))

(define (grid-spec n)
  (let* ((cols (max 1 (inexact->exact (ceiling (sqrt n)))))
         (rows (max 1 (inexact->exact (ceiling (/ n cols))))))
    (let vloop ((r rows))
      (if (<= r 1)
          (chain-spec 'horizontal cols)
          (list 'vsplit (/ 1 r)
                (chain-spec 'horizontal cols)
                (vloop (- r 1)))))))

(define (expose-enter!)
  "Saves the live layout and tiles this head's windows one per frame in
a numbered grid. Returns the window count, or #f with an echo when
there is nothing to expose. Callers arm the pick keymap."
  (let ((ids (tree-window-ids %frame-tree)))
    (if (null? ids)
        (begin (echo "no windows") #f)
        (begin
          (set! %expose-saved (dump-frames))
          (let* ((tree (spec->tree (grid-spec (length ids))))
                 (leaves (frame-leaves tree)))
            (resize-subtree! tree %last-output-x %last-output-y
                             %last-output-w %last-output-h)
            (let loop ((ids ids) (ls leaves))
              (unless (null? ids)
                (set-frame-window-ids! (car ls) (list (car ids)))
                (set-frame-current-window! (car ls) (car ids))
                (loop (cdr ids) (cdr ls))))
            (set! %frame-tree tree)
            (set! %current-frame (car leaves))
            (sync-frames!)
            (show-frame-overlays!)
            (length ids))))))

(define (expose-pick! n)
  "Leaves expose mode: restores the saved layout, then focuses the
window that was shown in grid cell N (#f = just restore)."
  (let* ((leaves (frame-leaves %frame-tree))
         (id (and n (< n (length leaves))
                  (frame-current-window (list-ref leaves n)))))
    (clear-frame-overlays!)
    (when %expose-saved
      (restore-frames! %expose-saved)
      (set! %expose-saved #f))
    (when id (focus-window-by-id! id))))

;; ---------------------------------------------------------------------
;; Unmaximize + gravity (StumpWM unmaximize / gravity): an unmaximized
;; window is shown at 2/3 of its frame, positioned by its gravity, but
;; stays a tiled window of that frame.
;; ---------------------------------------------------------------------

;; id -> gravity symbol, present iff the window is unmaximized.
(define %unmaximized (make-hash-table))

(define %gravities
  '(center top bottom left right
    top-left top-right bottom-left bottom-right))

(define (window-unmaximized? id)
  "Returns true when tiled window ID is using an unmaximized gravity."
  (and (hash-ref %unmaximized id) #t))

(define (clear-unmaximized! id)
  "Removes window ID's unmaximized gravity state."
  (hash-remove! %unmaximized id))

(define (unmaximized-rect frame id)
  (let* ((r (frame-display-rect frame))
         (x (car r)) (y (cadr r)) (w (caddr r)) (h (cadddr r))
         (uw (max 100 (quotient (* 2 w) 3)))
         (uh (max 80 (quotient (* 2 h) 3)))
         (grav (or (hash-ref %unmaximized id) 'center))
         (gx (cond ((memq grav '(left top-left bottom-left)) x)
                   ((memq grav '(right top-right bottom-right)) (- (+ x w) uw))
                   (else (+ x (quotient (- w uw) 2)))))
         (gy (cond ((memq grav '(top top-left top-right)) y)
                   ((memq grav '(bottom bottom-left bottom-right)) (- (+ y h) uh))
                   (else (+ y (quotient (- h uh) 2))))))
    (list gx gy uw uh)))

(define (unmaximize!)
  "Toggles the focused tiled window between filling its frame and a 2/3
rect positioned by its gravity (StumpWM unmaximize)."
  (let ((id (focused-window-id)))
    (cond
     ((not id) (echo "no window"))
     ((window-floating? id) (echo "window is floating"))
     ((hash-ref %unmaximized id)
      (hash-remove! %unmaximized id)
      (sync-frames!)
      (echo (format #f "~a fills its frame again" (window-title id))))
     (else
      (hash-set! %unmaximized id 'center)
      (sync-frames!)
      (echo (format #f "~a unmaximized (gravity: center)" (window-title id)))))))

(define (set-window-gravity! grav)
  "Sets the focused unmaximized window's gravity (StumpWM gravity)."
  (let ((id (focused-window-id)))
    (cond
     ((not id) (echo "no window"))
     ((not (memq grav %gravities))
      (echo (format #f "unknown gravity ~a" grav)))
     ((not (hash-ref %unmaximized id))
      (echo "unmaximize first (Print P u)"))
     (else
      (hash-set! %unmaximized id grav)
      (sync-frames!)
      (echo (format #f "gravity: ~a" grav))))))

;; ---------------------------------------------------------------------
;; Focus cycling
;; ---------------------------------------------------------------------

(define (focus-next-frame!)
  "Cycles %current-frame to the next leaf frame in tree order."
  (clear-float-focus!)
  (let* ((leaves (frame-leaves %frame-tree))
         (n (length leaves))
         (idx (list-index (lambda (f) (eq? f %current-frame)) leaves)))
    (when (and idx (> n 1))
      (set! %current-frame (list-ref leaves (modulo (+ idx 1) n)))))
  (sync-frames!))

(define (focus-next-window-in-frame!)
  "Cycles the current window shown within the current frame ('other
window')."
  (clear-float-focus!)
  (let* ((ids (frame-window-ids %current-frame))
         (n (length ids)))
    ;; n = 1 with nothing shown (after clear-current-frame!) should re-show it too.
    (when (and (> n 0) (or (> n 1) (not (frame-current-window %current-frame))))
      (let* ((cur (frame-current-window %current-frame))
             ;; cur can be #f after clear-current-frame!; start from the front then.
             (idx (or (list-index (lambda (i) (equal? i cur)) ids) -1)))
        (set-frame-current-window! %current-frame (list-ref ids (modulo (+ idx 1) n))))))
  (sync-frames!))

;; All window ids in the active group, in frame order. This is the
;; "buffer list" that focus-next-window!/pull-hidden-next! cycle through.
(define (all-window-ids)
  "Returns all active-group window identifiers in frame then float order."
  (append (append-map frame-window-ids (active-leaves))
          (group-floats %active-group)))

;; The leaf frame a window currently lives in (any head of the active
;; group), or #f.
(define (frame-of-window id)
  (find (lambda (f) (member id (frame-window-ids f)))
        (active-leaves)))

;; The head id whose tree holds window ID in the active group, or #f.
(define (head-of-window id)
  "Returns the active-group head containing tiled window ID, or #f."
  (if (find (lambda (f) (member id (frame-window-ids f)))
            (frame-leaves %frame-tree))
      %current-head-id
      (let ((found #f))
        (hash-for-each
         (lambda (hid pair)
           (when (and (not found)
                      (find (lambda (f) (member id (frame-window-ids f)))
                            (frame-leaves (car pair))))
             (set! found hid)))
         (group-heads %active-group))
        found)))

(define (focus-next-window!)
  "StumpWM's `next`: cycles through ALL windows of the group (the way
emacs cycles buffers), not just the current frame's stack. Focus moves to
wherever the next window lives -- its frame becomes current and the window
is raised in it if it was hidden."
  (let* ((ids (all-window-ids))
         (n (length ids))
         (cur (focused-window-id)))
    (when (> n 0)
      (let* ((idx (or (and cur (list-index (lambda (i) (equal? i cur)) ids)) -1))
             (next-id (list-ref ids (modulo (+ idx 1) n))))
        ;; focus-window-by-id! handles a window on another head.
        (focus-window-by-id! next-id))))
  (sync-frames!))

;; Windows not currently visible: everything that isn't its frame's
;; current window.
(define (hidden-window-ids)
  (append-map
   (lambda (f)
     (filter (lambda (id) (not (equal? id (frame-current-window f))))
             (frame-window-ids f)))
   (active-leaves)))

(define (pull-hidden-next!)
  "StumpWM's `pull` (pull-hidden-next): moves the next HIDDEN window of
the group into the current frame and shows it. Never steals a window that
is visible in another frame; a no-op when nothing is hidden."
  (let ((hidden (hidden-window-ids)))
    (unless (null? hidden)
      (let* ((id (car hidden))
             (f (frame-of-window id)))
        (set-frame-window-ids! f (delete id (frame-window-ids f)))
        (when (equal? (frame-current-window f) id)
          (set-frame-current-window!
           f (if (null? (frame-window-ids f)) #f (car (frame-window-ids f)))))
        (frame-add-window! %current-frame id)
        (sync-frames!)))))

(define (pull-window-from-other-frame!)
  "Finds the next window (round-robin, starting just after the current
frame) not already in the current frame, and moves it into the current
frame as its new current window. A no-op if there is only one frame or no
other frame holds any window."
  (let* ((leaves (frame-leaves %frame-tree))
         (n (length leaves))
         (idx (list-index (lambda (f) (eq? f %current-frame)) leaves)))
    (when (and idx (> n 1))
      (let loop ((i 1))
        (when (<= i (- n 1))
          (let ((candidate (list-ref leaves (modulo (+ idx i) n))))
            (if (pair? (frame-window-ids candidate))
                (let ((id (frame-current-window candidate)))
                  (set-frame-window-ids! candidate (delete id (frame-window-ids candidate)))
                  (set-frame-current-window!
                   candidate
                   (if (null? (frame-window-ids candidate)) #f (car (frame-window-ids candidate))))
                  (frame-add-window! %current-frame id))
                (loop (+ i 1))))))))
  (sync-frames!))

(define (focus-previous-frame!)
  "Cycles %current-frame to the previous leaf frame in tree order."
  (clear-float-focus!)
  (let* ((leaves (frame-leaves %frame-tree))
         (n (length leaves))
         (idx (list-index (lambda (f) (eq? f %current-frame)) leaves)))
    (when (and idx (> n 1))
      (set! %current-frame (list-ref leaves (modulo (- idx 1) n)))))
  (sync-frames!))

(define (focus-previous-window-in-frame!)
  "Cycles the current frame's shown window backwards."
  (clear-float-focus!)
  (let* ((ids (frame-window-ids %current-frame))
         (n (length ids)))
    (when (and (> n 0) (or (> n 1) (not (frame-current-window %current-frame))))
      (let* ((cur (frame-current-window %current-frame))
             (idx (or (list-index (lambda (i) (equal? i cur)) ids) 1))) ; #f -> wrap to last
        (set-frame-current-window! %current-frame (list-ref ids (modulo (- idx 1) n))))))
  (sync-frames!))

(define (focus-previous-window!)
  "StumpWM's `prev`: focus-next-window! backwards through the group."
  (let* ((ids (all-window-ids))
         (n (length ids))
         (cur (focused-window-id)))
    (when (> n 0)
      (let* ((idx (or (and cur (list-index (lambda (i) (equal? i cur)) ids)) 1))
             (prev-id (list-ref ids (modulo (- idx 1) n))))
        (focus-window-by-id! prev-id))))
  (sync-frames!))

(define (pull-hidden-previous!)
  "StumpWM's pull-hidden-previous: like pull-hidden-next! but takes the
last hidden window instead of the first."
  (let ((hidden (hidden-window-ids)))
    (unless (null? hidden)
      (let* ((id (last hidden))
             (f (frame-of-window id)))
        (set-frame-window-ids! f (delete id (frame-window-ids f)))
        (when (equal? (frame-current-window f) id)
          (set-frame-current-window!
           f (if (null? (frame-window-ids f)) #f (car (frame-window-ids f)))))
        (frame-add-window! %current-frame id)
        (sync-frames!)))))

;; ---------------------------------------------------------------------
;; Window marks (StumpWM mark / pull-marked): tag several windows, then
;; pull them all into the current frame at once.
;; ---------------------------------------------------------------------

(define %marked-windows '())

(define (marked-windows)
  "Returns the identifiers of currently marked windows."
  %marked-windows)

(define (mark-window-toggle!)
  "Toggles the mark on the current window and echoes the result."
  (let ((id (focused-window-id)))
    (when id
      (if (member id %marked-windows)
          (begin (set! %marked-windows (delete id %marked-windows))
                 (echo (format #f "unmarked ~a" (window-title id))))
          (begin (set! %marked-windows (cons id %marked-windows))
                 (echo (format #f "marked ~a" (window-title id))))))))

(define (clear-marks!)
  "Clears every window mark and reports the change."
  (set! %marked-windows '())
  (echo "marks cleared"))

(define (unmark-window! id)
  "Removes the mark from window ID."
  (set! %marked-windows (delete id %marked-windows)))

(define (pull-marked!)
  "Pulls every marked window of the active group into the current frame
and clears the marks."
  (let ((here (filter (lambda (id) (member id (all-window-ids)))
                      %marked-windows)))
    (if (null? here)
        (echo "no marked windows")
        (begin
          (for-each
           (lambda (id)
             (if (window-floating? id)
                 ;; Pulling a marked float = unfloat into the frame.
                 (when (remove-float! %active-group id)
                   (frame-add-window! %current-frame id))
                 (let ((f (frame-of-window id)))
                   (unless (eq? f %current-frame)
                     (take-window-out! f id)
                     (frame-add-window! %current-frame id)))))
           (reverse here))
          (set! %marked-windows
                (lset-difference equal? %marked-windows here))
          (sync-frames!)))))

;; ---------------------------------------------------------------------
;; Last-window / last-frame toggles (StumpWM other-window / fother)
;; ---------------------------------------------------------------------

(define (other-window!)
  "Toggles to the group's previously focused window (emacs C-x b RET)."
  (let ((lw (group-last-window %active-group)))
    (if (and lw (member lw (all-window-ids)))
        (focus-window-by-id! lw)
        (echo "no other window"))))

(define (other-frame!)
  "Toggles to the previously focused frame (StumpWM fother)."
  (clear-float-focus!)
  (let ((lf (group-last-frame %active-group)))
    (when (and lf (memq lf (frame-leaves %frame-tree))
               (not (eq? lf %current-frame)))
      (set! %current-frame lf)
      (sync-frames!))))

;; ---------------------------------------------------------------------
;; Directional navigation (StumpWM move-focus / move-window /
;; exchange-direction)
;; ---------------------------------------------------------------------

;; The leaf frame adjacent to %current-frame in direction DIR ('left
;; 'right 'up 'down), or #f at the edge. The tree tiles exactly, so
;; adjacency is exact edge equality; among frames sharing that edge the
;; one with the largest perpendicular overlap wins.
(define (frame-in-direction dir)
  (directional-neighbor
   %current-frame (frame-leaves %frame-tree) dir
   #:rectangle (lambda (frame)
                 (list (frame-x frame) (frame-y frame)
                       (frame-w frame) (frame-h frame)))))

(define (move-focus! dir)
  "Focuses the frame in direction DIR; at a screen edge, crosses to the
next head in that direction if there is one."
  (clear-float-focus!)
  (let ((target (frame-in-direction dir)))
    (cond
     (target
      (set! %current-frame target)
      (sync-frames!))
     ((head-in-direction dir) => focus-head!))))

;; Removes ID from FRAME's list, promoting the next window if it was
;; current. (The window is expected to be re-added elsewhere.)
(define (take-window-out! frame id)
  (set-frame-window-ids! frame (delete id (frame-window-ids frame)))
  (when (equal? (frame-current-window frame) id)
    (set-frame-current-window!
     frame
     (if (null? (frame-window-ids frame)) #f (car (frame-window-ids frame))))))

(define (move-window! dir)
  "Moves the current window into the frame in direction DIR (crossing to
the next head at a screen edge) and follows it with the focus."
  (let ((target (frame-in-direction dir))
        (id (current-frame-window)))
    (cond
     ((not id) #f)
     (target
      (take-window-out! %current-frame id)
      (frame-add-window! target id)
      (set! %current-frame target)
      (sync-frames!))
     ((head-in-direction dir)
      => (lambda (hid)
           (take-window-out! %current-frame id)
           (focus-head! hid)
           (frame-add-window! %current-frame id)
           (sync-frames!))))))

(define (exchange-windows! dir)
  "Swaps the current window with the one shown in the frame in direction
DIR (StumpWM exchange-direction). With an empty neighbor this is just a
move; focus follows the current window."
  (let ((target (frame-in-direction dir))
        (id (current-frame-window)))
    (when (and target id)
      (let ((other (frame-current-window target))
            (source %current-frame))
        (take-window-out! source id)
        (frame-add-window! target id)
        (when other
          (take-window-out! target other)
          (frame-add-window! source other))
        (set! %current-frame target)
        (sync-frames!)))))

;; ---------------------------------------------------------------------
;; Frame commands: only / fclear / split-equally
;; ---------------------------------------------------------------------

(define (collapse-to-one-frame!)
  "Collapses the tree to one full-area frame keeping every window, the
current one visible (StumpWM only)."
  (apply-layout-spec! 'leaf))

(define (clear-current-frame!)
  "Hides the current frame's shown window (it stays in the frame's list;
the frame shows empty -- StumpWM fclear)."
  (set-frame-current-window! %current-frame #f)
  (sync-frames!))

;; Replaces the current frame by N equal frames in a row/column. The
;; current frame's windows stay in the first of them.
(define (split-equally! orientation n)
  (when (> n 1)
    (let* ((f %current-frame)
           (rect (list (frame-x f) (frame-y f) (frame-w f) (frame-h f)))
           (first-leaf (make-frame 0 0 1 1 (frame-window-ids f)
                                   (frame-current-window f)))
           (subtree
            (let chain ((k n) (leaf first-leaf))
              (if (= k 1)
                  leaf
                  (make-split orientation (/ 1 k) leaf
                              (chain (- k 1) (make-frame 0 0 1 1 '() #f)))))))
      (if (eq? f %frame-tree)
          (set! %frame-tree subtree)
          (let-values (((parent side) (find-parent %frame-tree f)))
            (if (eq? side 'a)
                (set-split-child-a! parent subtree)
                (set-split-child-b! parent subtree))))
      (apply resize-subtree! subtree rect)
      (set! %current-frame first-leaf)
      (sync-frames!))))

(define (hsplit-equally! n)
  "Replaces the current frame with N equally wide frames."
  (split-equally! 'horizontal n))
(define (vsplit-equally! n)
  "Replaces the current frame with N equally tall frames."
  (split-equally! 'vertical n))

(define (close-current-window!)
  "Requests the focused window (float or current frame's window) be
closed."
  (let ((id (focused-window-id)))
    (when id
      (wm-close-window id))))

;; ---------------------------------------------------------------------
;; Sync: push frame geometry + focus down to Rust
;; ---------------------------------------------------------------------

;; Windows not currently shown in their frame are parked off-screen rather
;; than left with stale on-screen geometry.
(define %offscreen-x -10000)
(define %offscreen-y -10000)

;; The focus border (drawn by Rust *inside* the frame rect, see
;; BORDER_WIDTH in src/render.rs) must not overlap window content, so
;; windows are placed inset by this much within their frame.
(define %border-width 3)

;; Gaps: %inner-gap of empty space between adjacent frames (each frame
;; gives up half at a shared edge), %outer-gap between frames and the
;; usable-area boundary. Both default to 0 (off); enable from init.scm
;; with e.g. (set-gaps! 8 8).
(define %inner-gap 0)
(define %outer-gap 0)

(define (set-gaps! inner outer)
  "Sets the inner (between frames) and outer (screen edge) gap in pixels
and re-syncs the active group."
  (set! %inner-gap inner)
  (set! %outer-gap outer)
  (sync-frames!))

;; The rectangle a frame actually displays as (focus border drawn on it,
;; window inside it): the frame's tree rect shrunk by half the inner gap
;; on every side, except sides on the usable-area boundary which get the
;; outer gap instead.
(define (frame-display-rect frame)
  (let* ((x (frame-x frame)) (y (frame-y frame))
         (w (frame-w frame)) (h (frame-h frame))
         (half (quotient %inner-gap 2))
         (l (if (= x %last-output-x) %outer-gap half))
         (t (if (= y %last-output-y) %outer-gap half))
         (r (if (= (+ x w) (+ %last-output-x %last-output-w)) %outer-gap half))
         (b (if (= (+ y h) (+ %last-output-y %last-output-h)) %outer-gap half)))
    (list (+ x l) (+ y t) (max 1 (- w l r)) (max 1 (- h t b)))))

;; Called (when set) at the end of every sync-frames! -- (minde
;; groups) uses it to keep the status-line file for external bars (eww)
;; current without frames.scm having to know about groups or bars.
(define %sync-hook #f)
(define (set-sync-hook! proc)
  "Sets PROC to run after every completed frame synchronization."
  (set! %sync-hook proc))

(define (sync-frames-now!)
  "Walks every head's frame tree of the active group, placing each
frame's current window at its frame's pixel geometry, moving every other
(hidden) window off-screen, and setting input focus to the current
frame's current window."
  (for-each
   (lambda (rect+tree)
     ;; Bind the gap/outer-edge bounds to this tree's head while its
     ;; frames are placed (frame-display-rect reads %last-output-*).
     (let ((saved (list %last-output-x %last-output-y
                        %last-output-w %last-output-h))
           (r (car rect+tree)))
       (set! %last-output-x (car r)) (set! %last-output-y (cadr r))
       (set! %last-output-w (caddr r)) (set! %last-output-h (cadddr r))
       (for-each
        (lambda (frame)
          (let ((cur (frame-current-window frame))
                (rect (frame-display-rect frame)))
            (for-each
             (lambda (id)
               (cond
                ((not (equal? id cur))
                 (wm-place-window id %offscreen-x %offscreen-y (frame-w frame) (frame-h frame)))
                ;; Unmaximized: 2/3 rect by gravity, placed without the
                ;; tiled states (wm-place-float) so CSD corners return.
                ((hash-ref %unmaximized id)
                 (apply rust-call 'wm-place-float id (unmaximized-rect frame id)))
                (else
                 (let ((bw %border-width))
                   (wm-place-window id
                                    (+ (car rect) bw) (+ (cadr rect) bw)
                                    (- (caddr rect) (* 2 bw)) (- (cadddr rect) (* 2 bw)))))))
             (frame-window-ids frame))))
        (frame-leaves (cdr rect+tree)))
       (set! %last-output-x (car saved)) (set! %last-output-y (cadr saved))
       (set! %last-output-w (caddr saved)) (set! %last-output-h (cadddr saved))))
   (active-head-trees))
  ;; Tell Rust where the selected frame is, so the focus border marks the
  ;; frame itself (visible even when the frame is empty).
  (let ((rect (frame-display-rect %current-frame)))
    (apply wm-focus-rect rect))
  (let ((id (focused-window-id)))
    (if id
        (wm-focus-window id)
        ;; Empty current frame: drop keyboard focus so a hidden/unmapped
        ;; window doesn't keep receiving keys.
        (wm-clear-focus)))
  ;; Floats go on top of everything the placement/focus steps raised;
  ;; always-on-top windows above even those.
  (place-floats!)
  (raise-ontop!)
  ;; Remember the previous focus for the other-window!/other-frame!
  ;; toggles: whatever was shown at the end of the last sync becomes
  ;; "last" the moment something else is shown.
  (let ((g %active-group)
        (cur (current-frame-window)))
    (when cur (clear-urgent! cur))
    (let ((shown (group-shown-window g)))
      (unless (equal? shown cur)
        (when shown (set-group-last-window! g shown))
        (run-event-hook! 'focus-window cur)))
    (set-group-shown-window! g cur)
    (let ((shownf (group-shown-frame g)))
      (unless (eq? shownf %current-frame)
        (when shownf (set-group-last-frame! g shownf))
        (run-event-hook! 'focus-frame
                    (frame-x %current-frame) (frame-y %current-frame)
                    (frame-w %current-frame) (frame-h %current-frame))))
    (set-group-shown-frame! g %current-frame))
  (when %sync-hook (%sync-hook)))

(define (sync-frames!)
  "Like sync-frames-now!, but a no-op while a fullscreen window is active,
so incidental re-syncs (hooks, geometry echoes) don't fight the
fullscreen geometry. Leaving fullscreen clears the flag first and then
syncs, restoring the frame layout."
  (unless %fullscreen-window
    (sync-frames-now!)))

;; ---------------------------------------------------------------------
;; Fullscreen, force kill, pointer control, urgency
;; ---------------------------------------------------------------------

;; Window id currently fullscreen, or #f. Only ever one at a time.
(define %fullscreen-window #f)

(define (fullscreen-window)
  "Returns the fullscreen window identifier, or #f."
  %fullscreen-window)

(define (fullscreen!)
  "Toggles fullscreen on the current window (StumpWM fullscreen). While
active the frame layout is frozen; toggling off re-syncs it."
  (if %fullscreen-window
      (begin
        (rust-call 'wm-set-fullscreen %fullscreen-window #f)
        (set! %fullscreen-window #f)
        (sync-frames!))
      (let ((id (focused-window-id)))
        (if id
            (begin
              (set! %fullscreen-window id)
              (rust-call 'wm-set-fullscreen id #t))
            (echo "No window to fullscreen")))))

(define (clear-fullscreen-if-window! id)
  "Drops the fullscreen flag if ID owns it (window unmapped) and re-syncs."
  (when (equal? %fullscreen-window id)
    (set! %fullscreen-window #f)
    (sync-frames!)))

(define (kill-current-window!)
  "Force-kills the current window's client connection (StumpWM
kill-window) -- vs. close-current-window!'s polite xdg close."
  (let ((id (focused-window-id)))
    (if id
        (rust-call 'wm-kill-window id)
        (echo "No window to kill"))))

(define (ratwarp! x y)
  "Warps the pointer to global position X Y (StumpWM ratwarp)."
  (rust-call 'wm-warp-pointer x y))

(define (move-pointer-to-corner!)
  "Warps the pointer to the bottom-right corner of the usable area
(StumpWM banish)."
  (ratwarp! (- (+ %last-output-x %last-output-w) 2)
            (- (+ %last-output-y %last-output-h) 2)))

(define (current-frame-rect)
  "The current frame's (x y w h), for the frame-flash indicator."
  (list (frame-x %current-frame) (frame-y %current-frame)
        (frame-w %current-frame) (frame-h %current-frame)))

;; Urgent windows (xdg-activation requests), oldest first. Focusing a
;; window clears it (see sync-frames-now!); next-urgent! lives in
;; (minde groups) since the window may be parked in a hidden group.
(define %urgent-windows '())

(define (urgent-windows)
  "Returns urgent window identifiers in notification order."
  %urgent-windows)

(define (add-urgent-window! id)
  "Records ID as urgent, fires the 'urgent-window hook, and echoes it.
Called from Rust as (handle-urgent-window! id) via init.scm."
  (unless (member id %urgent-windows)
    (set! %urgent-windows (append %urgent-windows (list id))))
  ;; Urgency can arrive without any geometry/focus change, so explicitly
  ;; notify the status publisher installed as the sync hook.
  (when %sync-hook (%sync-hook))
  (run-event-hook! 'urgent-window id)
  (echo (string-append "Urgent: " (or (window-title id)
                                      (number->string id)))))

(define (clear-urgent! id)
  "Removes window ID from the urgent-window set."
  (set! %urgent-windows (delete id %urgent-windows)))

;; ---------------------------------------------------------------------
;; Event hooks
;; ---------------------------------------------------------------------
;;
;; These are no longer the direct handle-window-map!/handle-window-unmap!/
;; handle-output-geometry! hooks Rust looks up by name -- (minde groups)
;; owns those now (it needs to route to the active group and search
;; hidden groups on unmap). These handle-* procedures are the
;; single-group-tree half of that work.

(define (track-window-map! id title app-id)
  "Adds ID to the active group's current frame as its new current window
-- or floats it right away if the active group is a float group."
  (remember-window-title! id title app-id)
  (assign-window-number! id (group-all-trees %active-group))
  (if (group-float? %active-group)
      (begin
        (add-float! %active-group id (default-float-rect))
        (set! %focused-float id))
      (frame-add-window! %current-frame id))
  (run-event-hook! 'new-window id title app-id)
  (sync-frames!))

(define (track-float-map! id title app-id)
  "Like track-window-map!, but floats the new window immediately (a
#:float? placement rule matched) instead of adding it to the current
frame -- the frame's window keeps its place and focus geometry."
  (remember-window-title! id title app-id)
  (assign-window-number! id (group-all-trees %active-group))
  (run-event-hook! 'new-window id title app-id)
  (float-window! id))

(define (track-window-unmap! id)
  "Removes ID from the active group's tree, if present there. Returns #t
if found (and re-syncs), #f otherwise -- callers should then search other
groups' trees themselves via remove-window-from-tree-in!."
  (remove-window-from-active-tree! id))

(define (update-output-geometry! x y width height)
  "Single-head compatibility shim: treats the whole world as one head
with id 0. (minde groups)' handle-output-geometry! routes here only for
old binaries/tests; multi-head backends call handle-heads-change!."
  (when (and (> width 0) (> height 0))
    (heads-changed! (list (list 0 x y width height)) '())))

;;; frames.scm -- StumpWM-style manual frame tiling.
;;;
;;; Owns all layout policy: a binary frame tree, a "current frame" pointer,
;;; and the sync step that pushes computed geometries down to Rust via
;;; `wm-place-window` / `wm-focus-window`.
;;;
;;; Important design constraint (see tests/frames-test.scm): nothing at
;;; module load time calls any `wm-*` Rust subr. Those are only ever called
;;; from `sync-frames!`, `wm-on-window-map`, etc. -- i.e. in response to an
;;; event, never as a side effect of loading this file. That's what lets the
;;; unit test stub the `wm-*` procedures *before* loading this module and
;;! still have everything work.

(define-module (minde frames)
  #:use-module (srfi srfi-1)
  #:use-module (srfi srfi-9)
  #:use-module (srfi srfi-11)
  #:export (split-frame-horizontal!
            split-frame-vertical!
            remove-split!
            focus-next-frame!
            focus-next-window!
            focus-next-window-in-frame!
            pull-hidden-next!
            pull-window-from-other-frame!
            sync-frames!
            handle-window-map!
            handle-window-unmap!
            handle-output-geometry!
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
            forget-window-title!
            window-ids-with-titles
            focus-window-by-id!
            ;; Group-tree plumbing, used by (minde groups). frames.scm
            ;; keeps owning the <group> record and the "which group's tree
            ;; is currently live in %frame-tree/%current-frame" swap
            ;; (activate-group!) since that's intimately tied to the frame
            ;; internals below; (minde groups) owns the ordered group
            ;; list and higher-level operations (switch-to-group! etc).
            make-group
            group?
            group-name
            set-group-name!
            group-tree
            set-group-tree!
            group-current-frame
            set-group-current-frame!
            make-empty-group
            current-group
            current-tree
            current-output-size
            activate-group!
            park-group-windows!
            group-window-count
            frame-leaves
            frame-window-ids
            frame-add-window!
            hide-window!))

;; ---------------------------------------------------------------------
;; Data types
;; ---------------------------------------------------------------------

;; A leaf frame: a rectangle plus the list of window ids assigned to it and
;; which one (if any) is currently shown.
(define-record-type <frame>
  (make-frame x y w h window-ids current-window)
  frame?
  (x frame-x set-frame-x!)
  (y frame-y set-frame-y!)
  (w frame-w set-frame-w!)
  (h frame-h set-frame-h!)
  (window-ids frame-window-ids set-frame-window-ids!)
  (current-window frame-current-window set-frame-current-window!))

;; An internal split node. ORIENTATION is 'horizontal (side-by-side, a
;; vertical dividing line) or 'vertical (stacked, a horizontal dividing
;; line) -- matching StumpWM's terms where "vsplit" stacks frames
;; vertically. RATIO is child-a's share of the split (0 < ratio < 1).
(define-record-type <split>
  (make-split orientation ratio child-a child-b)
  split?
  (orientation split-orientation)
  (ratio split-ratio set-split-ratio!)
  (child-a split-child-a set-split-child-a!)
  (child-b split-child-b set-split-child-b!))

;; A group: a name plus its own frame tree and current-frame pointer.
;; Exactly one group is "active" at a time -- its tree/current-frame are
;; the live %frame-tree/%current-frame below; every other group's state
;; sits quiescently in its own record until (minde groups) activates
;; it. See activate-group!.
(define-record-type <group>
  (make-group name tree current-frame)
  group?
  (name group-name set-group-name!)
  (tree group-tree set-group-tree!)
  (current-frame group-current-frame set-group-current-frame!))

;; ---------------------------------------------------------------------
;; State
;; ---------------------------------------------------------------------

;; The root of the frame tree -- either a <frame> or a <split> whose leaves
;; are (transitively) <frame>s. This is always the ACTIVE group's tree;
;; other groups' trees live in their own <group> record until activated.
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

(define (current-output-size) (list %last-output-w %last-output-h))

;; The group whose tree/current-frame are currently loaded into
;; %frame-tree/%current-frame above. (minde groups) bootstraps its
;; default group list by grabbing this one and renaming it, so nothing
;; here needs to know about groups plural.
(define %active-group (make-group "default" %frame-tree %current-frame))

(define (current-group) %active-group)

(define (current-tree) %frame-tree)

;; Creates a fresh, empty, single-frame group of the given size. Building
;; a <group>/<frame> record is pure data construction -- no wm-* calls --
;; so this is safe to use at module load time too.
(define (make-empty-group name w h)
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
  (unless (eq? g %active-group)
    (flush-active-group!)
    (set! %frame-tree (group-tree g))
    (set! %current-frame (group-current-frame g))
    (set! %active-group g)
    (resize-subtree! %frame-tree %last-output-x %last-output-y
                     %last-output-w %last-output-h)))

;; Moves every window tracked by G (active or not) off-screen, without
;; touching focus. Used when a group is about to be hidden.
(define (park-group-windows! g)
  (let ((tree (if (eq? g %active-group) %frame-tree (group-tree g))))
    (for-each
     (lambda (frame)
       (for-each
        (lambda (id) (wm-place-window id %offscreen-x %offscreen-y (frame-w frame) (frame-h frame)))
        (frame-window-ids frame)))
     (frame-leaves tree))))

;; Total window count tracked by G (active or not).
(define (group-window-count g)
  (let ((tree (if (eq? g %active-group) %frame-tree (group-tree g))))
    (apply + (map (lambda (f) (length (frame-window-ids f))) (frame-leaves tree)))))

;; Parks a single window off-screen -- used when a window is moved out of
;; the active group's tree into a hidden group (so it doesn't linger
;; on-screen with stale geometry; see move-window-to-next-group! in
;; (minde groups)).
(define (hide-window! id)
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
(define (rust-call name . args)
  (let* ((mod (resolve-module '(guile-user) #:ensure #f))
         (var (and mod (module-variable mod name))))
    (if var
        (apply (variable-ref var) args)
        (begin
          (format #t "minde: ~a unbound, ignoring call~%" name)
          #f))))

(define (wm-place-window id x y w h) (rust-call 'wm-place-window id x y w h))

(define (echo text)
  "Shows TEXT in the compositor's message overlay (StumpWM's message
window), falling back to the log when running against a binary without
wm-message (or under the test stubs)."
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
  (if (frame? node)
      (list node)
      (append (frame-leaves (split-child-a node))
              (frame-leaves (split-child-b node)))))

;; Finds the parent <split> of LEAF within NODE, or #f if LEAF is NODE
;; itself or not found. Returns (values parent side) where side is 'a or
;; 'b, or (values #f #f).
(define (find-parent node leaf)
  (cond
   ((frame? node) (values #f #f))
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

;; id -> title, remembered from handle-window-map! for the windowlist and
;; `info`-style echoes. Survives moves between frames/groups; dropped on
;; unmap.
(define %window-titles (make-hash-table))

(define (window-title id)
  (or (hash-ref %window-titles id)
      (format #f "window ~a" id)))

(define (forget-window-title! id)
  (hash-remove! %window-titles id))

(define (remember-window-title! id title app-id)
  (hash-set! %window-titles id
             (if (and (string? title) (not (string-null? title)))
                 title
                 (if (string? app-id) app-id ""))))

(define (window-ids-with-titles)
  "All windows of the active group, as (id . title) pairs in frame order."
  (map (lambda (id) (cons id (window-title id))) (all-window-ids)))

(define (focus-window-by-id! id)
  "Jumps to window ID wherever it lives in the active group: its frame
becomes current and it is raised. Used by the fuzzel windowlist."
  (let ((f (frame-of-window id)))
    (when f
      (set! %current-frame f)
      (set-frame-current-window! f id)
      (sync-frames!))))

(define (frame-add-window! frame id)
  (set-frame-window-ids! frame (append (frame-window-ids frame) (list id)))
  (set-frame-current-window! frame id))

;; Removes ID from every frame of TREE it appears in (there should be at
;; most one). Returns #t if it was found and removed. Generic over any
;; tree so (minde groups) can search hidden groups' trees too.
(define (remove-window-from-tree-in! tree id)
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

;; Removes ID from the active group's tree only. Returns #t if found, in
;; which case the active group is re-synced.
(define (remove-window-from-active-tree! id)
  (let ((found (remove-window-from-tree-in! %frame-tree id)))
    (when found (sync-frames!))
    found))

(define (current-frame-window)
  (frame-current-window %current-frame))

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
                (if (frame? sibling)
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
  (if (frame? node)
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
  (if (frame? node)
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
    (unless (frame? node)
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

(define (dump-layout-spec)
  "The active group's live tree as a layout spec; ratios are derived from
the actual pixel rectangles so manual resizes survive a dump/apply
round-trip."
  (let node->spec ((node %frame-tree))
    (if (frame? node)
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

;; ---------------------------------------------------------------------
;; Focus cycling
;; ---------------------------------------------------------------------

(define (focus-next-frame!)
  "Cycles %current-frame to the next leaf frame in tree order."
  (let* ((leaves (frame-leaves %frame-tree))
         (n (length leaves))
         (idx (list-index (lambda (f) (eq? f %current-frame)) leaves)))
    (when (and idx (> n 1))
      (set! %current-frame (list-ref leaves (modulo (+ idx 1) n)))))
  (sync-frames!))

(define (focus-next-window-in-frame!)
  "Cycles the current window shown within the current frame ('other
window')."
  (let* ((ids (frame-window-ids %current-frame))
         (n (length ids)))
    (when (> n 1)
      (let* ((cur (frame-current-window %current-frame))
             (idx (list-index (lambda (i) (equal? i cur)) ids)))
        (set-frame-current-window! %current-frame (list-ref ids (modulo (+ idx 1) n))))))
  (sync-frames!))

;; All window ids in the active group, in frame order. This is the
;; "buffer list" that focus-next-window!/pull-hidden-next! cycle through.
(define (all-window-ids)
  (append-map frame-window-ids (frame-leaves %frame-tree)))

;; The leaf frame a window currently lives in, or #f.
(define (frame-of-window id)
  (find (lambda (f) (member id (frame-window-ids f)))
        (frame-leaves %frame-tree)))

(define (focus-next-window!)
  "StumpWM's `next`: cycles through ALL windows of the group (the way
emacs cycles buffers), not just the current frame's stack. Focus moves to
wherever the next window lives -- its frame becomes current and the window
is raised in it if it was hidden."
  (let* ((ids (all-window-ids))
         (n (length ids))
         (cur (current-frame-window)))
    (when (> n 0)
      (let* ((idx (or (and cur (list-index (lambda (i) (equal? i cur)) ids)) -1))
             (next-id (list-ref ids (modulo (+ idx 1) n)))
             (f (frame-of-window next-id)))
        (when f
          (set! %current-frame f)
          (set-frame-current-window! f next-id)))))
  (sync-frames!))

;; Windows not currently visible: everything that isn't its frame's
;; current window.
(define (hidden-window-ids)
  (append-map
   (lambda (f)
     (filter (lambda (id) (not (equal? id (frame-current-window f))))
             (frame-window-ids f)))
   (frame-leaves %frame-tree)))

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

(define (close-current-window!)
  "Requests the current frame's current window be closed."
  (let ((id (current-frame-window)))
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
(define (set-sync-hook! proc) (set! %sync-hook proc))

(define (sync-frames!)
  "Walks the frame tree, placing each frame's current window at its frame's
pixel geometry, moving every other (hidden) window off-screen, and setting
input focus to the current frame's current window."
  (for-each
   (lambda (frame)
     (let ((cur (frame-current-window frame))
           (rect (frame-display-rect frame)))
       (for-each
        (lambda (id)
          (if (equal? id cur)
              (let ((bw %border-width))
                (wm-place-window id
                                 (+ (car rect) bw) (+ (cadr rect) bw)
                                 (- (caddr rect) (* 2 bw)) (- (cadddr rect) (* 2 bw))))
              (wm-place-window id %offscreen-x %offscreen-y (frame-w frame) (frame-h frame))))
        (frame-window-ids frame))))
   (frame-leaves %frame-tree))
  ;; Tell Rust where the selected frame is, so the focus border marks the
  ;; frame itself (visible even when the frame is empty).
  (let ((rect (frame-display-rect %current-frame)))
    (apply wm-focus-rect rect))
  (let ((id (current-frame-window)))
    (if id
        (wm-focus-window id)
        ;; Empty current frame: drop keyboard focus so a hidden/unmapped
        ;; window doesn't keep receiving keys.
        (wm-clear-focus)))
  (when %sync-hook (%sync-hook)))

;; ---------------------------------------------------------------------
;; Event hooks
;; ---------------------------------------------------------------------
;;
;; These are no longer the direct wm-on-window-map/wm-on-window-unmap/
;; wm-on-output-geometry hooks Rust looks up by name -- (minde groups)
;; owns those now (it needs to route to the active group and search
;; hidden groups on unmap). These handle-* procedures are the
;; single-group-tree half of that work.

(define (handle-window-map! id title app-id)
  "Adds ID to the active group's current frame as its new current window."
  (remember-window-title! id title app-id)
  (frame-add-window! %current-frame id)
  (sync-frames!))

(define (handle-window-unmap! id)
  "Removes ID from the active group's tree, if present there. Returns #t
if found (and re-syncs), #f otherwise -- callers should then search other
groups' trees themselves via remove-window-from-tree-in!."
  (remove-window-from-active-tree! id))

(define (handle-output-geometry! x y width height)
  "Called on output init/resize and whenever layer-shell exclusive zones
change the usable area (X Y is its origin -- e.g. below a docked bar).
Resizes the active group's frame tree to the new rect and remembers it so
newly activated groups get the current geometry too."
  (when (and (> width 0) (> height 0))
    (set! %last-output-x x)
    (set! %last-output-y y)
    (set! %last-output-w width)
    (set! %last-output-h height)
    (resize-subtree! %frame-tree x y width height)
    (sync-frames!)))

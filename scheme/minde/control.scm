;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Versioned desktop inspection and validated, receipted actions.

(define-module (minde control)
  #:use-module (srfi srfi-1)
  #:use-module (srfi srfi-13)
  #:use-module (ice-9 match)
  #:use-module (ice-9 format)
  #:use-module (minde commands)
  #:use-module (minde compositor model)
  #:use-module (minde compositor frames)
  #:use-module (minde compositor rust)
  #:use-module (minde groups)
  #:use-module ((minde layouts) #:prefix layout:)
  #:export (desktop-snapshot
            new-action-request
            perform-action!
            action-status
            events-since
            note-desktop-change!
            register-control-commands!))

;; These survive init.scm reloads. An identifier is an epoch, not a secret.
(define %session
  (let ((now (gettimeofday)))
    (format #f "~x-~x-~x" (getpid) (car now) (cdr now))))
(define %revision 0)
(define %fingerprint #f)
(define %receipt-limit 256)
(define %issued 0)
(define %receipt-floor 0)
(define %receipts (make-hash-table))
(define %performing? #f)

(define (note-desktop-change! . ignored)
  "Invalidates observed revisions after a policy synchronization or event.
This counter also detects a supported change followed by its inverse."
  (set! %revision (+ %revision 1)))

(define (groups)
  ;; Read the policy owner's records, never maintain another group mirror.
  (module-ref (resolve-module '(minde groups)) '%groups))

(define (group-by-name name)
  (find (lambda (g) (string=? name (group-name g))) (groups)))

(define (group-ids g)
  (delete-duplicates
   (append (append-map frame-window-ids
                       (append-map frame-leaves (group-all-trees g)))
           (group-floats g))))

(define (window-group id)
  (find (lambda (g) (member id (group-ids g))) (groups)))

(define (group-head-trees g)
  (sort
   (cons (cons (if (eq? g (current-group))
                   (current-head-id) (group-loaded-head g))
               (if (eq? g (current-group)) (current-tree) (group-tree g)))
         (hash-map->list (lambda (id pair) (cons id (car pair))) (group-heads g)))
   (lambda (a b) (< (car a) (car b)))))

(define (rect frame)
  (vector (frame-x frame) (frame-y frame) (frame-w frame) (frame-h frame)))

(define (native-tree-spec tree)
  (if (frame-node? tree) 'leaf
      (list (if (eq? (split-orientation tree) 'horizontal) 'hsplit 'vsplit)
            (split-ratio tree)
            (native-tree-spec (split-child-a tree))
            (native-tree-spec (split-child-b tree)))))

(define (frame-facts g)
  (list->vector
   (append-map
    (lambda (entry)
      (map (lambda (f index)
             `((head . ,(car entry)) (index . ,index) (geometry . ,(rect f))
               (windows . ,(list->vector (frame-window-ids f)))
               (selected-window . ,(or (frame-current-window f) 'null))
               (focused . ,(and (eq? g (current-group))
                                (eq? f (current-frame))))))
           (frame-leaves (cdr entry))
           (iota (length (frame-leaves (cdr entry))))))
    (group-head-trees g))))

(define (window-facts g id redact?)
  (let* ((locations
          (append-map
           (lambda (entry)
             (filter-map
              (lambda (f index)
                (and (member id (frame-window-ids f))
                     `((head . ,(car entry)) (frame . ,index)
                       (selected . ,(equal? id (frame-current-window f))))))
              (frame-leaves (cdr entry))
              (iota (length (frame-leaves (cdr entry))))))
           (group-head-trees g)))
         (geometry (rust-call-if-bound 'wm-window-geometry id))
         (visible? (and (eq? g (current-group))
                        (or (window-floating? id)
                            (any (lambda (x) (assq-ref x 'selected)) locations))))
         (fullscreen? (equal? id (fullscreen-window))))
    `((id . ,id) (group . ,(group-name g))
      ,@(if redact? '()
            `((title . ,(window-title id)) (app-id . ,(or (window-app-id id) ""))))
      (locations . ,(list->vector locations))
      (focused . ,(equal? id (focused-window-id)))
      (visible . ,(and visible? #t))
      (floating . ,(and (window-floating? id) #t))
      (fullscreen . ,fullscreen?)
      (urgent . ,(and (member id (urgent-windows)) #t))
      (geometry . ,(if (and geometry visible?) (list->vector geometry) 'null))
      (float-geometry . ,(let ((r (float-geometry id)))
                          (if r (list->vector r) 'null))))))

(define (desktop-body redact?)
  `((focused-group . ,(current-group-name))
    (focused-window . ,(or (focused-window-id) 'null))
    (focused-head . ,(current-head-id))
    (head-mode . ,(head-mode))
    (locked . ,(and (rust-call-if-bound 'wm-session-locked?) #t))
    (groups . ,(list->vector
                (map (lambda (g)
                       `((name . ,(group-name g))
                         (focused . ,(eq? g (current-group)))
                         (floating . ,(group-float? g))
                         (dynamic . ,(and (dynamic-group? g) #t))
                         (trees . ,(list->vector
                                    (map (lambda (entry)
                                           `((head . ,(car entry))
                                             (scheme . ,(format #f "~s" (native-tree-spec (cdr entry))))))
                                         (group-head-trees g))))
                         (frames . ,(frame-facts g))))
                     (groups))))
    (windows . ,(list->vector
                 (append-map
                  (lambda (g)
                    (map (lambda (id) (window-facts g id redact?))
                         (sort (group-ids g) <)))
                  (groups))))
    (outputs . ,(list->vector
                 (map (lambda (h)
                        `((id . ,(car h))
                          (geometry . ,(list->vector (take (cdr h) 4)))
                          (name . ,(if (> (length h) 5) (list-ref h 5) ""))))
                      (or (rust-call-if-bound 'wm-outputs) (heads)))))
    (layouts . ,(list->vector (layout:layout-names)))
    (layout-definitions . ,(list->vector
                            (map (lambda (name)
                                   `((name . ,name)
                                     (scheme . ,(format #f "~s" (layout:layout-spec name)))))
                                 (layout:layout-names))))))

(define (refresh-revision! body)
  ;; Native committed geometry can change and change back between reads.
  ;; Its owner epoch covers that case without calling Scheme from drag locks.
  (let ((fingerprint (call-with-output-string
                       (lambda (p)
                         (write (list body (rust-call-if-bound 'wm-geometry-revision)) p)))))
    (unless (equal? fingerprint %fingerprint)
      (set! %fingerprint fingerprint)
      (set! %revision (+ %revision 1)))
    %revision))

(define* (desktop-snapshot #:key (redact? #f))
  "Returns schema-v1 desktop facts across all groups, a session epoch,
revision, and event cursor. Objects are alists; collections are vectors.
Null denotes unavailable information. Locking forces content redaction."
  (let* ((body (desktop-body #f))
         (revision (refresh-revision! body))
         (redact? (or redact? (assq-ref body 'locked))))
    `((schema-version . 1) (session . ,%session) (revision . ,revision)
      (event-sequence . ,(or (rust-call-if-bound 'wm-event-cursor) 0))
      (redacted . ,(and redact? #t))
      ,@(if redact? (desktop-body #t) body))))

(define (fail code . details)
  (throw 'control-error code details))

(define (require-window id)
  (or (window-group id) (fail 'unknown-window `((window . ,id)))))

(define (require-group name)
  (or (group-by-name name) (fail 'unknown-group `((group . ,name)))))

(define (focus-target! id)
  (let ((g (require-window id)))
    (switch-to-group! (group-name g))
    ;; Switching runs user hooks; they may remove or move the target.
    (unless (eq? (require-window id) (current-group))
      (fail 'target-changed `((window . ,id))))
    (unless (focus-window-by-id! id)
      (fail 'target-unavailable `((window . ,id))))
    (unless (equal? id (focused-window-id))
      (fail 'postcondition-failed `((window . ,id))))
    `((focused-window . ,id))))

(define (move-target! id name)
  (let ((from (require-window id)) (to (require-group name)))
    ((module-ref (resolve-module '(minde groups)) 'move-window-between-groups!)
     id from to)
    (sync-frames!)
    (unless (eq? (require-window id) to)
      (fail 'postcondition-failed `((window . ,id))))
    `((window . ,id) (group . ,name))))

(define (float-target! id enabled)
  (require-window id)
  (unless (eq? (and (window-floating? id) #t) enabled)
    (focus-target! id)
    (if enabled (float-window! id) (unfloat-window! id)))
  (unless (eq? (and (window-floating? id) #t) enabled)
    (fail 'postcondition-failed `((window . ,id))))
  `((window . ,id) (floating . ,enabled)))

(define (fullscreen-target! id enabled)
  (require-window id)
  (unless (eq? (equal? id (fullscreen-window)) enabled)
    (when (fullscreen-window) (fullscreen!))
    (when enabled
      (focus-target! id)
      (fullscreen!)))
  (unless (eq? (equal? id (fullscreen-window)) enabled)
    (fail 'postcondition-failed `((window . ,id))))
  `((window . ,id) (fullscreen . ,enabled)))

(define (select-frame! name head index)
  (let* ((g (require-group name))
         (entry (assv head (group-head-trees g))))
    (unless (and entry (< index (length (frame-leaves (cdr entry)))))
      (fail 'unknown-frame `((group . ,name) (head . ,head) (frame . ,index))))
    (when (dynamic-group? g) (fail 'dynamic-layout `((group . ,name))))
    (switch-to-group! name)
    (focus-head! head)
    (unless (and (eq? g (current-group)) (= head (current-head-id)))
      (fail 'target-changed))
    (let ((target (list-ref (frame-leaves (cdr entry)) index)))
      (focus-frame-by-index! index)
      (unless (and (eq? g (current-group)) (= head (current-head-id))
                   (eq? target (current-frame)))
        (fail 'target-changed)))))

(define (validate-targets! name arguments)
  ;; Validate existing object references before setting the execution marker.
  ;; Wrappers repeat the checks after hooks, immediately before each mutation.
  (let ((window (assq-ref arguments 'window))
        (group (assq-ref arguments 'group)))
    (when (and window (memq name '(focus-window! move-window-to-group!
                                  set-window-floating! set-window-fullscreen!
                                  close-window! screenshot)))
      (unless (>= window 0) (fail 'invalid-argument))
      (require-window window))
    (when (memq name '(switch-group! move-window-to-group! split-frame! apply-layout!))
      (require-group group))
    (when (memq name '(split-frame! apply-layout!))
      (let* ((g (require-group group))
             (entry (assv (assq-ref arguments 'head) (group-head-trees g)))
             (index (if (eq? name 'split-frame!) (assq-ref arguments 'frame) 0)))
        (unless (and entry (< index (length (frame-leaves (cdr entry)))))
          (fail 'unknown-frame))
        (when (dynamic-group? g) (fail 'dynamic-layout))))
    (when (and (eq? name 'apply-layout!)
               (not (layout:layout-spec (assq-ref arguments 'layout))))
      (fail 'unknown-layout))
    (when (eq? name 'screenshot)
      (unless (string-prefix? "/" (assq-ref arguments 'path)) (fail 'invalid-path))
      (when (and window (not (rust-call-if-bound 'wm-window-geometry window)))
        (fail 'target-unavailable)))))

(define (split-target! name head index direction)
  (select-frame! name head index)
  (let ((g (current-group))
        (before (length (frame-leaves (current-tree)))))
    (case direction
      ((horizontal) (split-frame-horizontal!))
      ((vertical) (split-frame-vertical!)))
    ;; The split synchronizes and runs hooks. A replacement group's tree can
    ;; coincidentally have the expected number of leaves, so count alone does
    ;; not establish that the explicitly selected target remains current.
    (unless (and (eq? g (current-group)) (= head (current-head-id))
                 (= (+ before 1) (length (frame-leaves (current-tree)))))
      (fail 'postcondition-failed))
    `((group . ,name) (head . ,head) (frame-count . ,(+ before 1)))))

(define (layout-target! name head layout)
  (unless (layout:layout-spec layout) (fail 'unknown-layout `((layout . ,layout))))
  (select-frame! name head 0)
  (let ((wanted (layout:layout-spec layout)) (g (current-group)))
    (layout:apply-layout! layout)
    (unless (and (eq? g (current-group)) (= head (current-head-id))
                 (equal? wanted (native-tree-spec (current-tree))))
      (fail 'postcondition-failed)))
  `((group . ,name) (head . ,head) (layout . ,layout)))

(define (close-target! id)
  (require-window id)
  (unless (rust-call-if-bound 'wm-close-window id)
    (fail 'request-rejected `((window . ,id))))
  `((window . ,id) (completion . window-disappeared)))

(define (screenshot-target! path window)
  (unless (string-prefix? "/" path) (fail 'invalid-path `((path . ,path))))
  (when window
    (require-window window)
    (unless (rust-call-if-bound 'wm-window-geometry window)
      (fail 'target-unavailable `((window . ,window)))))
  (let ((token (if window (rust-call-if-bound 'wm-screenshot path window)
                  (rust-call-if-bound 'wm-screenshot path))))
    (unless token (fail 'request-rejected))
    `((token . ,token) (operation . screenshot) (path . ,path))))

(define (register-control-commands!)
  "Registers the typed desktop control surface in the ordinary command registry.
Loading this module has no compositor side effects; call after legacy setup."
  (define (register name procedure parameters scope effects completion summary example)
    (register-command! name procedure #:parameters parameters #:category 'control
                       #:summary summary #:documentation summary
                       #:result-schema 'alist #:scope scope #:effects effects
                       #:completion completion #:retry 'receipt #:demo-id 'non-visual
                       #:examples (list example)))
  (define window '((name . window) (type . integer) (minimum . 0)))
  (define group '((name . group) (type . string) (min-length . 1)))
  (define head '((name . head) (type . integer) (minimum . 0)))
  (define enabled '((name . enabled) (type . boolean)))
  (register 'desktop-snapshot (lambda () (desktop-snapshot)) '()
            'desktop '() 'immediate "Inspect every group, window, frame, and output."
            '(desktop-snapshot))
  (register 'windows (lambda () `((windows . ,(assq-ref (desktop-snapshot) 'windows))))
            '() 'desktop '() 'immediate "List windows across all groups."
            '(perform-action! 'windows '()))
  (register 'focus-window! focus-target! (list window) 'window '(focus group head)
            'immediate "Focus a window across groups and heads."
            '(perform-action! 'focus-window! '((window . 42))))
  (register 'switch-group!
            (lambda (name)
              (let ((g (require-group name)))
                (switch-to-group! name)
                (unless (eq? g (current-group)) (fail 'postcondition-failed))
                `((group . ,name))))
            (list group) 'group '(focus group) 'immediate "Switch to an existing group by its exact name."
            '(perform-action! 'switch-group! '((group . " II "))))
  (register 'move-window-to-group! move-target! (list window group) 'window
            '(placement focus) 'immediate "Move a window to an existing group without following."
            '(perform-action! 'move-window-to-group! '((window . 42) (group . " II "))))
  (register 'set-window-floating! float-target! (list window enabled) 'window
            '(floating placement focus group) 'immediate "Set floating state; focus the target when changing it."
            '(perform-action! 'set-window-floating! '((window . 42) (enabled . #t))))
  (register 'set-window-fullscreen! fullscreen-target! (list window enabled) 'window
            '(fullscreen focus group) 'immediate "Set fullscreen state; enabling replaces any fullscreen owner."
            '(perform-action! 'set-window-fullscreen! '((window . 42) (enabled . #t))))
  (register 'split-frame! split-target!
            (list group head '((name . frame) (type . integer) (minimum . 0))
                  '((name . direction) (type . (enum horizontal vertical))))
            'frame '(layout focus group head) 'immediate "Split an explicitly selected manual frame."
            '(perform-action! 'split-frame! '((group . " I ") (head . 0) (frame . 0) (direction . horizontal))))
  (register 'apply-layout! layout-target!
            (list group head '((name . layout) (type . string) (min-length . 1)))
            'head '(layout focus group head) 'immediate "Apply a named layout to a manual group's head."
            '(perform-action! 'apply-layout! '((group . " I ") (head . 0) (layout . "wide"))))
  (register 'close-window! close-target! (list window) 'window '(close)
            'window-disappeared "Request a client close; completion means its window disappeared."
            '(perform-action! 'close-window! '((window . 42))))
  (register 'screenshot screenshot-target!
            (list '((name . path) (type . string) (min-length . 1))
                  '((name . window) (type . (maybe integer)) (default . #f)))
            'desktop '(file) 'automation "Queue a PNG capture. Observe completion with action-status or mindectl wait using the returned request-id."
            '(perform-action! 'screenshot '((path . "/tmp/minde.png")))))

(define (result id status value error)
  `((schema-version . 1) (session . ,%session) (request-id . ,(or id 'null))
    (status . ,status) (revision . ,%revision)
    (value . ,value) ,@(if error `((error . ,error)) '())))

(define (error-result id code stage details)
  ;; User commands can throw the same condition keys as the dispatcher, with
  ;; arbitrary irritants. Error receipts need the same writable-data guarantee
  ;; as successful ones; an opaque diagnostic must never poison later replay.
  (let* ((id (and (string? id) (<= (string-length id) 512) id))
         (code (if (and (symbol? code)
                        (<= (string-length (symbol->string code)) 128))
                   code 'execution-failed))
         (make-error
          (lambda (details)
            (result id 'error 'null
                    `((code . ,code) (stage . ,stage)
                      (effects-may-have-occurred . ,(not (eq? stage 'validation)))
                      (details . ,details))))))
    (catch #t
      (lambda ()
        (if (and (rust-bound? 'ipc-writable-datum)
                 (rust-bound? 'ipc-normalize-datum) (rust-bound? 'ipc-print-datum))
            (freeze-result (make-error (rust-call 'ipc-writable-datum details)))
            (make-error '((diagnostics-unavailable . #t)))))
      (lambda _ (make-error '((diagnostics-exceeded-reply-budget . #t)))))))

(define (freeze-result response)
  ;; Use the actual bounded wire encoder before publishing a receipt. A
  ;; writable result alone is insufficient: the envelope must fit too.
  ;; Reading it back also detaches receipts from user-owned mutable data.
  (let* ((normalized (rust-call 'ipc-normalize-datum response))
         (text (rust-call 'ipc-print-datum (list 'ok normalized))))
    (with-fluids ((read-eval? #f))
      (cadr (call-with-input-string text read)))))

(define (receipt-number id)
  (and (string? id)
       (string-prefix? (string-append %session ":") id)
       (let* ((text (substring id (+ 1 (string-length %session))))
              (n (and (not (string-null? text)) (string-every char-numeric? text)
                      (string->number text))))
         (and (exact-integer? n) (> n 0) (<= n %issued) n))))

(define (new-action-request)
  "Allocates a server-issued receipt ID and returns session/revision preconditions.
The last 256 allocations are retained. Older IDs remain recognizably expired;
they cannot execute again. Allocate before transmitting a retryable action."
  (desktop-snapshot)
  (set! %issued (+ %issued 1))
  (let ((floor (max 0 (- %issued %receipt-limit))))
    (do ((n (+ %receipt-floor 1) (+ n 1))) ((> n floor))
      (hash-remove! %receipts n))
    (set! %receipt-floor floor))
  (hash-set! %receipts %issued (vector #f #f #f))
  `((session . ,%session) (revision . ,%revision)
    (request-id . ,(format #f "~a:~a" %session %issued))))

(define* (action-status id #:key (session %session))
  "Returns a stored action result, refreshing asynchronous completion.
An expired receipt or missing automation token reports outcome-unknown."
  (desktop-snapshot)
  (let* ((n (receipt-number id)) (entry (and n (hash-ref %receipts n))))
    (cond
     ((not (equal? session %session)) (error-result id 'session-mismatch 'validation '()))
     ((not n) (error-result id 'invalid-request-id 'validation '()))
     ((not entry) (error-result id 'outcome-unknown 'observation '()))
     ((not (vector-ref entry 1)) (result id 'ready 'null #f))
     (else
      (let* ((old (vector-ref entry 1)) (pending (vector-ref entry 2))
             (value (assq-ref old 'value))
             (updated
              (case pending
                ((window-disappeared)
                 (and (not (window-group (assq-ref value 'window)))
                      (result id 'applied value #f)))
                ((automation)
                 (let ((state (rust-call-if-bound 'wm-automation-status (assq-ref value 'token))))
                   (cond
                    ((not state) (error-result id 'outcome-unknown 'observation '()))
                    ((eq? (cadr state) 'pending) #f)
                    ((memq (cadr state) '(done accepted)) (result id 'applied value #f))
                    (else (error-result id 'operation-failed 'observation
                                        `((operation . ,(car state)) (status . ,(cadr state))))))))
                (else #f))))
        (when updated
          (vector-set! entry 1 updated)
          (vector-set! entry 2 #f))
        ;; A caller may annotate the returned alist in Scheme. Keep those
        ;; annotations detached from the authoritative stored receipt.
        (freeze-result (or updated old)))))))

(define* (perform-action! name arguments #:key (session %session)
                          (if-revision #f) (request-id #f))
  "Validates and executes a typed action, returning a versioned receipt.
Use new-action-request before sending if transport retries are needed. Raw
eval remains separate. Rejected requests consume their receipt too. A request
that cannot fit a bounded fingerprint stays observably rejected; subsequent
submissions with its ID conflict, so inspect action-status for that rejection.
This procedure never waits or silently retries."
  (let ((id request-id) (entry #f) (reserved? #f) (stage 'validation))
    (catch #t
      (lambda ()
        (desktop-snapshot)
        (unless (equal? session %session) (fail 'session-mismatch))
        (when %performing? (fail 'reentrant-action))
        (unless (rust-call-if-bound 'wm-control-ready?) (fail 'unsupported-context))
        (unless (and (rust-bound? 'ipc-normalize-datum) (rust-bound? 'ipc-print-datum))
          (fail 'serializer-unavailable))
        (unless id (set! id (assq-ref (new-action-request) 'request-id)))
        (let ((n (receipt-number id)))
          (unless n (fail 'invalid-request-id))
          (unless (> n %receipt-floor) (fail 'outcome-unknown))
          (set! entry (hash-ref %receipts n)))
        ;; Reserve before validation so even a malformed or oversized request
        ;; cannot leave a reusable, apparently unsubmitted receipt. The marker
        ;; deliberately never matches a printable request fingerprint.
        (unless (vector-ref entry 0)
          (set! reserved? #t)
          (vector-set! entry 0 'unfingerprintable-request))
        (when (and (not reserved?)
                   (eq? (vector-ref entry 0) 'unfingerprintable-request))
          (fail 'request-id-conflict))
        (let* ((validation-error #f)
               (command (command-ref name))
               (positional
                (catch #t
                  (lambda () (validate-command-arguments name arguments))
                  (lambda (key . args)
                    (set! validation-error (cons key args)) #f)))
               (fingerprint
                (rust-call 'ipc-print-datum
                           (rust-call 'ipc-normalize-datum
                                      (list name
                                            (if validation-error
                                                (list 'rejected-arguments arguments)
                                                (list 'validated-arguments positional))
                                            if-revision)))))
          (cond
           ((not reserved?)
            (unless (equal? fingerprint (vector-ref entry 0)) (fail 'request-id-conflict))
            (action-status id #:session session))
           (else
            (vector-set! entry 0 fingerprint)
            (when validation-error (apply throw validation-error))
            (unless (command-parameters command) (fail 'untyped-command))
            (unless (or (not if-revision)
                        (and (exact-integer? if-revision) (= if-revision %revision)))
              (fail 'stale-state `((expected . ,if-revision) (actual . ,%revision))))
            (validate-targets! name
                              (map cons (command-arguments command) positional))
            (vector-set! entry 1 (result id 'pending 'null #f))
            (let* ((before %fingerprint)
                   (value
                    (dynamic-wind
                      (lambda () (set! %performing? #t) (set! stage 'execution))
                      (lambda () (apply (command-procedure command) positional))
                      (lambda () (set! %performing? #f)))))
              (set! stage 'result-encoding)
              (set! value (rust-call 'ipc-normalize-datum value))
              (unless (or (eq? (command-result-schema command) 'unknown)
                          (schema-value-valid? (command-result-schema command) value))
                (fail 'invalid-result))
              ;; Completion policies have their own result contract even for
              ;; extensions declaring a broad result schema. Never store a
              ;; pending receipt that action-status cannot safely inspect.
              (let ((completion (command-completion command)))
                (when (memq completion '(automation window-disappeared))
                  (unless (schema-value-valid? 'alist value) (fail 'invalid-result))
                  (let ((target (assq-ref value (if (eq? completion 'automation)
                                                   'token 'window))))
                    (unless (and (exact-integer? target)
                                 (if (eq? completion 'automation)
                                     (<= 1 target #x7fffffffffffffff)
                                     (>= target 0)))
                      (fail 'invalid-result)))))
              (desktop-snapshot)
              (let* ((completion (command-completion command))
                     (pending? (memq completion '(automation window-disappeared)))
                     (status (cond (pending? 'pending)
                                   ((and (memq (command-procedure command)
                                               (list focus-target! move-target! float-target!
                                                     fullscreen-target! layout-target!))
                                         (equal? before %fingerprint)) 'unchanged)
                                   (else 'applied)))
                     (response (freeze-result (result id status value #f))))
                (vector-set! entry 1 response)
                (vector-set! entry 2 (and pending? completion))
                (freeze-result response)))))))
      (lambda (key . args)
        (let* ((known? (and (memq key '(control-error command-validation-error ipc-data-error))
                            (pair? args)))
               (code (if known? (car args) 'execution-failed))
               (details (if known? (if (pair? (cdr args)) (cadr args) '())
                            `((exception . ,key))))
               (response (error-result id code stage details)))
          ;; A conflicting duplicate must not overwrite the original receipt.
          (when (and entry reserved?)
            (vector-set! entry 1 (freeze-result response))
            (vector-set! entry 2 #f))
          response)))))

(define* (events-since sequence #:key (session %session) (redact? #f))
  "Returns sequenced journal events or an explicit resynchronization gap.
Query-time lock redaction also applies to events recorded before locking."
  (unless (and (exact-integer? sequence) (>= sequence 0))
    (fail 'invalid-argument))
  (let* ((reply (rust-call-if-bound 'wm-events-since sequence))
         (redact? (or redact? (rust-call-if-bound 'wm-session-locked?)))
         (gap? (or (not (equal? session %session)) (not reply) (assq-ref reply 'gap)))
         (events (and reply (not gap?) (assq-ref reply 'events))))
    `((schema-version . 1) (session . ,%session)
      (sequence . ,(if reply (assq-ref reply 'sequence) 0))
      (gap . ,gap?)
      (events . ,(list->vector
                   (filter-map
                    (lambda (entry)
                      (catch #t
                        (lambda ()
                          (let* ((datum (with-fluids ((read-eval? #f))
                                          (call-with-input-string (cadr entry)
                                            (lambda (p)
                                              (let ((datum (read p)))
                                                (unless (eof-object? (read p)) (fail 'invalid-event))
                                                datum)))))
                                 (name (car datum)) (args (cdr datum)))
                            (and (or (not redact?)
                                     (memq name '(focus-window focus-frame focus-group
                                                  destroy-window session-lock session-unlock
                                                  automation-result window-geometry
                                                  new-window window-title-changed)))
                                 `((sequence . ,(car entry)) (name . ,name)
                                   (arguments . ,(list->vector
                                                  (if (and redact? (memq name '(new-window window-title-changed)))
                                                      (list (car args) "" "") args)))))))
                        (lambda _ #f)))
                    (or events '())))))))

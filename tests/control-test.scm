;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Exercise the public control loop against real Scheme policy and Rust fakes.

(use-modules (srfi srfi-1))
(define %placements (make-hash-table))
(define %focused #f)
(define %closed '())
(define %captures '())
(define %automation (make-hash-table))
(define %next-token 0)
(define %locked? #f)
(define %ready? #t)
(define %cursor 0)
(define %journal '())
(define %gap? #f)
(define %geometry-revision 0)
(define (wm-geometry-revision) %geometry-revision)
(define (wm-place-window id x y w h)
  (hash-set! %placements id (list x y w h)) #t)
(define wm-place-float wm-place-window)
(define (wm-focus-window id) (set! %focused id) #t)
(define (wm-clear-focus) (set! %focused #f) #t)
(define (wm-close-window id) (set! %closed (cons id %closed)) #t)
(define (wm-set-floating id enabled) #t)
(define (wm-set-fullscreen id enabled) #t)
(define (wm-raise-window id) #t)
(define (wm-focus-rect . args) #t)
(define (wm-message . args) #t)
(define (wm-log text) #t)
(define (wm-output-geometry) '(0 0 1280 720))
(define %outputs '((0 0 0 1280 720 "headless")))
(define (wm-outputs) %outputs)
(define (wm-window-geometry id)
  (let ((r (hash-ref %placements id)))
    (and r (>= (car r) 0) (>= (cadr r) 0) r)))
(define (wm-session-locked?) %locked?)
(define (wm-control-ready?) %ready?)
(define (wm-event-cursor) %cursor)
(define (wm-events-since sequence)
  `((sequence . ,%cursor) (gap . ,%gap?)
    (events . ,(if %gap? '() (filter (lambda (e) (> (car e) sequence)) %journal)))))
(define (wm-screenshot path . window)
  (set! %next-token (+ %next-token 1))
  (set! %captures (cons (cons path window) %captures))
  (hash-set! %automation %next-token '(screenshot pending))
  %next-token)
(define (wm-automation-status token) (hash-ref %automation token))

(use-modules (minde commands) (minde control) (minde compositor frames)
             (minde groups) (minde hooks) (minde layouts))
(load-from-path "ipc-reply.scm")

(define (minde-mirror-event name args)
  (set! %cursor (+ %cursor 1))
  (set! %journal
        (append %journal
                (list (list %cursor
                            (call-with-output-string
                             (lambda (p) (write (cons name args) p))))))))
(set-sync-hook! note-desktop-change!)
(register-control-commands!)
(update-output-geometry! 0 0 1280 720)

(define failures 0)
(define checks 0)
(define (check description actual expected)
  (set! checks (+ checks 1))
  (unless (equal? actual expected)
    (set! failures (+ failures 1))
    (format #t "FAIL - ~a: expected ~s, got ~s~%" description expected actual)))
(define (check-true description value) (check description (and value #t) #t))
(define (status result) (assq-ref result 'status))
(define (code result) (assq-ref (assq-ref result 'error) 'code))
(define (window id)
  (find (lambda (w) (= id (assq-ref w 'id)))
        (vector->list (assq-ref (desktop-snapshot) 'windows))))
(define (frame-count group)
  (vector-length
   (assq-ref (find (lambda (g) (string=? group (assq-ref g 'name)))
                  (vector->list (assq-ref (desktop-snapshot) 'groups))) 'frames)))
(define* (act name arguments #:optional (request (new-action-request)))
  (perform-action! name arguments
                   #:session (assq-ref request 'session)
                   #:if-revision (assq-ref request 'revision)
                   #:request-id (assq-ref request 'request-id)))

;; Rust allocates window identifiers from zero; zero is a valid target, not
;; the absence marker (Scheme uses #f for that).
(handle-window-map! 0 "First mapped window" "zero-id")
(check "zero window ID remains visible in desktop facts" (assq-ref (window 0) 'id) 0)
(check "first Rust window is accepted by typed focus"
       (status (act 'focus-window! '((window . 0)))) 'unchanged)
(check "zero ID actually retains keyboard focus" %focused 0)
(check "zero window ID is accepted by targeted screenshot"
       (status (act 'screenshot '((path . "/tmp/zero-window.png") (window . 0)))) 'pending)
(check "zero window ID is accepted by close"
       (status (act 'close-window! '((window . 0)))) 'pending)
(check "native close receives zero without substitution" %closed '(0))
(handle-window-unmap! 0)
(set! %closed '())
(set! %captures '())

;; The native epoch detects a client geometry change followed by its inverse,
;; even when the eventual desktop body is identical and no Scheme hook ran.
(let ((request (new-action-request)))
  (set! %geometry-revision (+ %geometry-revision 2))
  (check "committed geometry reversal invalidates an observed revision"
         (code (act 'windows '() request)) 'stale-state))

;; Duplicate titles in separate groups must not merge identities.
(handle-window-map! 1 "Same title" "app-one")
(handle-window-map! 2 "Same title" "app-two")
(switch-to-group! " II ")
(handle-window-map! 3 "Same title" "app-three")
(check "snapshot inventories hidden and visible groups"
       (map (lambda (w) (assq-ref w 'id))
            (vector->list (assq-ref (desktop-snapshot) 'windows))) '(1 2 3))
(check "hidden window retains ownership" (assq-ref (window 1) 'group) " I ")
(check "hidden geometry is unavailable" (assq-ref (window 1) 'geometry) 'null)
(check "hidden window visibility is false" (assq-ref (window 1) 'visible) #f)
(let ((before (desktop-snapshot)))
  (check "observation does not invent a new revision"
         (assq-ref (desktop-snapshot) 'revision) (assq-ref before 'revision)))
(check "focus across group boundary applies"
       (status (act 'focus-window! '((window . 1)))) 'applied)
(check "cross-group action actually changes policy focus" (focused-window-id) 1)
(check "cross-group action actually changes Rust focus" %focused 1)
(check "cross-group focus activates owner's group" (current-group-name) " I ")
(check "repeating explicit focus is unchanged"
       (status (act 'focus-window! '((window . 1)))) 'unchanged)

;; Validation rejects calls before any mutation.
(for-each
 (lambda (arguments)
   (check "invalid named arguments are rejected"
          (code (act 'close-window! arguments)) 'invalid-argument))
 '(((window . "1")) ((window . -1)) ((window . 1) (unknown . #t))
   ((window . 1) (window . 2)) ()))
(check "invalid calls never send close requests" %closed '())
(check "missing target is explicit"
       (code (act 'focus-window! '((window . 999)))) 'unknown-window)
(set! %ready? #f)
(check "unsafe execution context is rejected"
       (code (act 'close-window! '((window . 1)))) 'unsupported-context)
(set! %ready? #t)
(check "context rejection sends no close" %closed '())
(let ((request (new-action-request)))
  (focus-window-by-id! 2)
  (focus-window-by-id! 1)
  (check "a mutation and its inverse still invalidate the old revision"
         (code (act 'close-window! '((window . 1)) request)) 'stale-state))
(check "stale revision sends no close" %closed '())
(let* ((request (new-action-request)) (id (assq-ref request 'request-id)))
  (focus-window-by-id! 2)
  (focus-window-by-id! 1)
  (check "stale call is rejected" (code (act 'close-window! '((window . 1)) request)) 'stale-state)
  (check "stale rejection is a terminal observable receipt"
         (code (action-status id)) 'stale-state)
  (check "retry of same stale invocation returns its original rejection"
         (code (act 'close-window! '((window . 1)) request)) 'stale-state)
  (check "stale rejected ID cannot be rebound to another invocation"
         (code (act 'close-window! '((window . 2)) request)) 'request-id-conflict)
  (check "conflicting retry preserves the original stale rejection"
         (code (action-status id)) 'stale-state))
(for-each
 (lambda (arguments expected)
   (let* ((request (new-action-request)) (id (assq-ref request 'request-id)))
     (check "validation failure is returned" (code (act 'close-window! arguments request)) expected)
     (check "validation rejection is observable by receipt ID" (code (action-status id)) expected)
     (check "identical rejected invocation replays rejection"
            (code (act 'close-window! arguments request)) expected)
     (check "different arguments cannot reuse rejected receipt"
            (code (act 'close-window! '((window . 1)) request)) 'request-id-conflict)
     (check "conflict does not overwrite rejection" (code (action-status id)) expected)))
 '(((window . "not-an-id")) ((window . 999))) '(invalid-argument unknown-window))
(check "validation receipt tests sent no close" %closed '())
(let ((request (new-action-request)))
  ;; This window is in a hidden group, so title changes do not need frame sync.
  (handle-window-title-change! 3 "Temporary title" "app-three")
  (handle-window-title-change! 3 "Same title" "app-three")
  (check "hidden title change and reversal invalidate preconditions"
         (code (act 'close-window! '((window . 1)) request)) 'stale-state))
(check "wrong compositor epoch rejects before mutation"
       (code (perform-action! 'close-window! '((window . 1)) #:session "other-session"))
       'session-mismatch)

;; Moves and setters verify real policy changes, including no-op setters.
(check "move across groups applies"
       (status (act 'move-window-to-group! '((window . 2) (group . " II ")))) 'applied)
(check "moved window belongs to destination" (group-has-window? " II " 2) #t)
(check "move does not follow" (current-group-name) " I ")
(check "float setter applies across groups"
       (status (act 'set-window-floating! '((window . 2) (enabled . #t)))) 'applied)
(check "float setter changed managed state" (window-floating? 2) #t)
(check "same float setting is unchanged"
       (status (act 'set-window-floating! '((window . 2) (enabled . #t)))) 'unchanged)
(check "fullscreen setter applies"
       (status (act 'set-window-fullscreen! '((window . 2) (enabled . #t)))) 'applied)
(check "disabling fullscreen for another target preserves owner"
       (status (act 'set-window-fullscreen! '((window . 3) (enabled . #f)))) 'unchanged)
(check "unrelated fullscreen disable keeps current fullscreen" (fullscreen-window) 2)
(act 'set-window-fullscreen! '((window . 2) (enabled . #f)))

;; A pending receipt is replayed without resending, even after state changes.
(let* ((request (new-action-request))
       (reply (act 'close-window! '((window . 3)) request))
       (id (assq-ref request 'request-id)))
  (check "close initially waits for observed disappearance" (status reply) 'pending)
  (check "close sent exactly once" %closed '(3))
  (check "duplicate pending close returns pending"
         (status (act 'close-window! '((window . 3)) request)) 'pending)
  (check "duplicate does not send another close" %closed '(3))
  (check "reusing request ID for different action conflicts"
         (code (act 'focus-window! '((window . 3)) request)) 'request-id-conflict)
  (handle-window-unmap! 3)
  (check "close becomes applied when window disappears" (status (action-status id)) 'applied)
  (check "duplicate completed request survives vanished target"
         (status (act 'close-window! '((window . 3)) request)) 'applied)
  (check "completion replay still does not resend" %closed '(3)))

;; Screenshot completion, failure, and eviction all have finite outcomes.
(for-each
 (lambda (terminal expected)
   (let* ((reply (act 'screenshot '((path . "/tmp/minde-control-test.png"))))
          (id (assq-ref reply 'request-id))
          (token (assq-ref (assq-ref reply 'value) 'token)))
     (check "screenshot starts pending" (status reply) 'pending)
     (if terminal
         (hash-set! %automation token (list 'screenshot terminal))
         (hash-remove! %automation token))
     (let ((observed (action-status id)))
       (check "screenshot terminal state is observable"
              (if (eq? expected 'applied) (status observed) (code observed)) expected))))
 '(done failed #f) '(applied operation-failed outcome-unknown))
(check "capture without absolute output path is rejected"
       (code (act 'screenshot '((path . "relative.png")))) 'invalid-path)
(check "rejected screenshot was never queued" (length %captures) 3)

;; Errors after effects must also be receipted and must not repeat effects.
(define %effect-count 0)
(register-command! 'effect-then-error!
 (lambda () (set! %effect-count (+ %effect-count 1)) (error "after effect"))
 #:parameters '() #:category 'test #:summary "Test receipt on exception"
 #:effects '(counter) #:completion 'immediate)
(let* ((request (new-action-request)) (reply (act 'effect-then-error! '() request)))
  (check "execution error records possible effects"
         (assq-ref (assq-ref reply 'error) 'effects-may-have-occurred) #t)
  (check "execution error replay stays an error"
         (code (act 'effect-then-error! '() request)) 'execution-failed)
  (check "execution error retry never repeats effects" %effect-count 1))

;; A custom command must not leave a receipt that is impossible to serialize.
(define %result-effects 0)
(for-each
 (lambda (name produce schema)
   (register-command! name
    (lambda () (set! %result-effects (+ %result-effects 1)) (produce))
    #:parameters '() #:category 'test #:summary "Test result validation"
    #:effects '(counter) #:result-schema schema #:completion 'immediate)
   (let* ((before %result-effects)
          (request (new-action-request)) (reply (act name '() request))
          (replay (act name '() request)))
     (check "invalid custom result becomes a readable action error" (status reply) 'error)
     (check "invalid result reports possible effects"
            (assq-ref (assq-ref reply 'error) 'effects-may-have-occurred) #t)
     (check "invalid result receipt itself remains serializable"
            (car (ipc-ok-reply reply)) 'ok)
     (check "invalid result replay retains failure" (status replay) 'error)
     (check "invalid result replay cannot repeat effects" %result-effects (+ before 1))))
 '(opaque-result! mistyped-result! oversized-result!)
 (list (lambda () (current-output-port)) (lambda () "not an integer")
       (lambda () (expt 2 %ipc-reply-max-chars)))
 '(any integer any))

;; Async extensions must return the identity needed by their observation
;; policy. Malformed values become terminal errors, not poisoned pendings.
(for-each
 (lambda (name completion value)
   (register-command! name
    (lambda () (set! %result-effects (+ %result-effects 1)) value)
    #:parameters '() #:category 'test #:summary "Malformed async result"
    #:result-schema 'any #:completion completion)
   (let* ((before %result-effects) (request (new-action-request))
          (reply (act name '() request)))
     (check "invalid async result is rejected before pending storage" (code reply) 'invalid-result)
     (check "invalid async result remains observable"
            (code (action-status (assq-ref request 'request-id))) 'invalid-result)
     (check "invalid async result replay is terminal"
            (code (act name '() request)) 'invalid-result)
     (check "invalid async replay does not repeat effects" %result-effects (+ before 1))))
 '(missing-token! false-token! oversized-token! missing-close-target! negative-close-target!)
 '(automation automation automation window-disappeared window-disappeared)
 (list '() '((token . #f)) `((token . ,(expt 2 64))) '() '((window . -1))))

(register-command! 'external-counter!
 (lambda () (set! %effect-count (+ %effect-count 1)) %effect-count)
 #:parameters '() #:category 'test #:summary "Change an effect outside desktop facts"
 #:effects '(counter) #:result-schema 'integer #:completion 'immediate)
(check "custom effects outside the snapshot still report execution"
       (status (act 'external-counter! '())) 'applied)

(for-each
 (lambda (condition)
   (let ((name (if (eq? condition 'control-error) 'opaque-control-error!
                   'opaque-validation-error!)))
     (register-command! name
      (lambda ()
        (set! %effect-count (+ %effect-count 1))
        (throw condition 'custom-failure (current-output-port)))
      #:parameters '() #:category 'test #:summary "Opaque custom exception"
      #:effects '(counter) #:completion 'immediate)
     (let* ((before %effect-count) (request (new-action-request))
            (reply (act name '() request)))
       (check "opaque custom error diagnostics remain writable"
              (car (ipc-ok-reply reply)) 'ok)
       (check "custom exception preserves stable error code" (code reply) 'custom-failure)
       (check "opaque diagnostic replay preserves the error"
              (code (act name '() request)) 'custom-failure)
       (check "opaque custom error replay does not repeat effects" %effect-count (+ before 1)))))
 '(control-error command-validation-error))

(register-command! 'empty-control-error!
 (lambda () (throw 'control-error))
 #:parameters '() #:category 'test #:summary "Malformed custom exception"
 #:completion 'immediate)
(check "malformed custom exception cannot break error handling"
       (code (act 'empty-control-error! '())) 'execution-failed)

(register-command! 'bounded-argument!
 (lambda (value) (set! %effect-count (+ %effect-count 1)) %effect-count)
 #:parameters '(((name . value) (type . any))) #:category 'test
 #:summary "Bound request fingerprint serialization" #:effects '(counter)
 #:result-schema 'integer #:completion 'immediate)
(let* ((before %effect-count) (request (new-action-request))
       (id (assq-ref request 'request-id)))
  (check "oversized request fingerprint is rejected before invocation"
         (code (act 'bounded-argument! `((value . ,(expt 2 %ipc-reply-max-chars))) request))
         'result-too-large)
  (check "oversized validated argument cannot reach the command" %effect-count before)
  (check "unfingerprintable rejection remains observable"
         (code (action-status id)) 'result-too-large)
  (check "unfingerprintable retransmission cannot bypass the terminal rejection"
         (code (act 'bounded-argument! `((value . ,(expt 2 %ipc-reply-max-chars))) request))
         'request-id-conflict)
  (check "unfingerprintable rejected ID cannot become a new valid invocation"
         (code (act 'bounded-argument! '((value . 1)) request)) 'request-id-conflict)
  (check "unfingerprintable receipt conflict never invokes command" %effect-count before))

(let* ((reply (act 'external-counter! '()))
       (id (assq-ref reply 'request-id))
       (expected (assq-ref reply 'value)))
  (set-cdr! (assq 'status reply) 'pending)
  (set-cdr! (assq 'value reply) 'altered)
  (let ((observed (action-status id)))
    (check "annotating a direct action result cannot mutate the stored receipt"
           (status observed) 'applied)
    (check "receipt retains its original value" (assq-ref observed 'value) expected)
    (set-cdr! (assq 'value observed) 'altered-again)
    (check "annotating queried receipt cannot mutate stored value"
           (assq-ref (action-status id) 'value) expected)))

;; Hooks can alter context during focus; do not split the replacement group.
(switch-to-group! " I ")
(split-frame-horizontal!)
(define (redirect-frame . args)
  (remove-event-hook! 'focus-frame redirect-frame)
  (switch-to-group! " II "))
(let ((destination-before (frame-count " II ")))
  (add-event-hook! 'focus-frame redirect-frame)
  (let ((reply (act 'split-frame!
                    '((group . " I ") (head . 0) (frame . 1) (direction . vertical)))))
    (remove-event-hook! 'focus-frame redirect-frame)
    (check "focus hook changing context aborts targeted split" (status reply) 'error)
    (check "replacement group was not split by mistake"
           (frame-count " II ") destination-before)))

;; Hooks also run after the split. A different tree with the expected count
;; must not masquerade as a successful postcondition for the requested group.
(switch-to-group! " II ")
(apply-layout-spec! '(hsplit 1/2 leaf leaf))
(switch-to-group! " I ")
(apply-layout-spec! 'leaf)
(let ((request (new-action-request)))
  (add-event-hook! 'focus-frame redirect-frame)
  (let ((reply (act 'split-frame!
                    '((group . " I ") (head . 0) (frame . 0) (direction . vertical))
                    request)))
    (remove-event-hook! 'focus-frame redirect-frame)
    (check "split hook switching to a same-count tree fails postcondition"
           (code reply) 'postcondition-failed)
    (check "post-split failure reports that effects occurred"
           (assq-ref (assq-ref reply 'error) 'effects-may-have-occurred) #t)
    (check "requested group was split before the hook switched away" (frame-count " I ") 2)
    (check "replacement group retained its existing layout" (frame-count " II ") 2)
    (check "retry returns the stored postcondition failure"
           (code (act 'split-frame!
                      '((group . " I ") (head . 0) (frame . 0) (direction . vertical))
                      request)) 'postcondition-failed)
    (check "retry after partial effects never splits again" (frame-count " I ") 2)))

;; Applying a layout fires a message hook; success must mean the layout stuck.
(switch-to-group! " I ")
(define-layout! "review-two" '(hsplit 1/2 leaf leaf))
(define (undo-layout . args)
  (remove-event-hook! 'message undo-layout)
  (apply-layout-spec! 'leaf))
(add-event-hook! 'message undo-layout)
(let ((reply (act 'apply-layout! '((group . " I ") (head . 0) (layout . "review-two")))))
  (remove-event-hook! 'message undo-layout)
  (check "layout changed by hook cannot claim the desired layout applied" (status reply) 'error))

;; Replay applies query-time privacy, including content recorded before lock.
(set! %journal '((1 "(new-window 8 \"Secret title\" \"secret.app\")")
                  (2 "(message \"Secret message\")")
                  (3 "(automation-result 7 screenshot done)")
                  (4 "(window-geometry 8 (10 20 640 480))")
                  (5 "(window-title-changed 8 \"Another secret\" \"secret.app\")")))
(set! %cursor 5)
(set! %locked? #t)
(let* ((snapshot (desktop-snapshot))
       (reply (events-since 0)) (events (vector->list (assq-ref reply 'events))))
  (check "lock forces snapshot redaction" (assq-ref snapshot 'redacted) #t)
  (check-true "locked snapshot omits titles and app IDs"
              (every (lambda (w) (and (not (assq 'title w)) (not (assq 'app-id w))))
                     (vector->list (assq-ref snapshot 'windows))))
  (check "locked replay suppresses content messages" (length events) 4)
  (check "locked replay retains committed geometry notifications"
         (assq-ref (list-ref events 2) 'arguments) '#(8 (10 20 640 480)))
  (check "pre-lock title is redacted during replay"
         (assq-ref (car events) 'arguments) '#(8 "" ""))
  (check "pre-lock title changes are also redacted"
         (assq-ref (last events) 'arguments) '#(8 "" ""))
  (check "redaction does not lose journal watermark" (assq-ref reply 'sequence) 5))
(set! %locked? #f)
(set! %gap? #t)
(check "journal gap requests explicit resynchronization"
       (assq-ref (events-since 0) 'gap) #t)
(check "journal gap cannot be mistaken for partial replay"
       (assq-ref (events-since 0) 'events) '#())
(set! %gap? #f)
(check "session mismatch requires a new snapshot"
       (assq-ref (events-since 0 #:session "other") 'gap) #t)
(check "session mismatch supplies no potentially misapplied events"
       (assq-ref (events-since 0 #:session "other") 'events) '#())
(define %reader-effect? #f)
(set! %journal '((1 "#.(begin (set! %reader-effect? #t) '(focus-window 1))")))
(set! %cursor 1)
(with-fluids ((read-eval? #t)) (events-since 0))
(check "replaying event data never invokes read-time evaluation" %reader-effect? #f)

;; Once an issued ID ages out it must never become a fresh request again.
(let* ((request (new-action-request))
       (id (assq-ref request 'request-id)) (before (length %closed)))
  (act 'close-window! '((window . 1)) request)
  (do ((i 0 (+ i 1))) ((= i 256)) (new-action-request))
  (check "expired receipt query reports unknown outcome"
         (code (action-status id)) 'outcome-unknown)
  (check "expired request ID cannot execute again"
         (code (act 'close-window! '((window . 1)) request)) 'outcome-unknown)
  (check "expiry never repeats close" (length %closed) (+ before 1)))

;; A desktop snapshot must retain head identity instead of flattening frames.
(set! %outputs '((0 0 0 1280 720 "left") (1 1280 0 800 600 "right")))
(handle-heads-change! (map (lambda (o) (take o 5)) %outputs))
(switch-to-group! " I ")
(focus-head! 1)
(handle-window-map! 11 "Second output" "multihead-test")
(focus-head! 0)
(check "desktop snapshot includes every enabled output"
       (vector-length (assq-ref (desktop-snapshot) 'outputs)) 2)
(check "window-to-frame location preserves its output head"
       (assq-ref (vector-ref (assq-ref (window 11) 'locations) 0) 'head) 1)
(check "typed focus crosses head boundary"
       (status (act 'focus-window! '((window . 11)))) 'applied)
(check "cross-head focus targets correct head" (current-head-id) 1)
(check "cross-head focus targets correct window" (focused-window-id) 11)
(check-true "geometry stays in desktop coordinates on second output"
            (>= (vector-ref (assq-ref (window 11) 'geometry) 0) 1280))

(format #t "control-test: ~a checks, ~a failures~%" checks failures)
(unless (zero? failures) (exit 1))

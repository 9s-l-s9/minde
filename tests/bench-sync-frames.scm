;;; bench-sync-frames.scm -- micro-benchmark of the frame sync hot path.
;;;
;;; Run with:
;;;   make bench-scheme
;;; or directly:
;;;   guile -L scheme tests/bench-sync-frames.scm [WINDOWS] [ITERATIONS]
;;;
;;; Stubs the Rust primitives with counters, builds a group of WINDOWS
;;; windows spread over a few splits, then times ITERATIONS calls of
;;; sync-frames-now! -- the work behind every focus change -- and prints
;;; the mean and maximum per call plus how many placements reached the
;;; (stubbed) compositor. Timing is reported, never asserted: this file is
;;; for measuring a change, not for gating one.

(use-modules (srfi srfi-1) (ice-9 format))

(define %placement-calls 0)
(define %batch-calls 0)

(define (wm-place-window id x y w h) (set! %placement-calls (+ 1 %placement-calls)) #t)
(define (wm-place-windows entries)
  (set! %batch-calls (+ 1 %batch-calls))
  (set! %placement-calls (+ (length entries) %placement-calls))
  '())
(define (wm-place-float id x y w h) #t)
(define (wm-focus-window id) #t)
(define (wm-clear-focus) #t)
(define (wm-focus-rect x y w h) #t)
(define (wm-raise-window id) #t)
(define (wm-set-floating id on) #t)
(define (wm-close-window id) #t)
(define (wm-output-geometry) (list 1920 1080))
(define (wm-outputs) (list (list 0 0 0 1920 1080 "bench")))
(define (wm-log msg) #t)
(define (wm-message text timeout) #t)

(use-modules (minde compositor frames) (minde groups))

(define windows (or (and (> (length (command-line)) 1)
                         (string->number (cadr (command-line))))
                    12))
(define iterations (or (and (> (length (command-line)) 2)
                            (string->number (caddr (command-line))))
                       2000))

(handle-heads-change! (list (list 0 0 0 1920 1080)))
(let loop ((i 1))
  (when (<= i windows)
    (handle-window-map! i (format #f "window ~a" i) "bench")
    (when (and (< i 5) (< i windows)) (split-frame-vertical!))
    (loop (+ i 1))))

(define (now-us)
  (let ((t (gettimeofday)))
    (+ (* 1000000 (car t)) (cdr t))))

;; Warm up (autocompile, caches), then measure.
(let loop ((i 0)) (when (< i 50) (focus-next-window!) (loop (+ i 1))))
(set! %placement-calls 0)
(set! %batch-calls 0)

(define max-us 0)
(define start (now-us))
(let loop ((i 0))
  (when (< i iterations)
    (let ((t0 (now-us)))
      (focus-next-window!)
      (set! max-us (max max-us (- (now-us) t0))))
    (loop (+ i 1))))
(define total-us (- (now-us) start))

(format #t "sync-frames bench: ~a windows, ~a focus changes~%" windows iterations)
(format #t "  mean ~,1f us/call, max ~a us~%" (/ total-us iterations) max-us)
(format #t "  placements sent ~a (~,2f per call), batch calls ~a~%"
        %placement-calls (/ %placement-calls iterations) %batch-calls)

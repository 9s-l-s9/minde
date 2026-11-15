#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Real Guile callbacks must reject bad arguments without partial effects or
# terminating the desktop, and hooks may return ordinary Scheme truth values.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh
export MINDE_TEST_PROTOCOLS=1
nested_output_directory "${MINDE_GUILE_BOUNDARY_E2E_OUT:-}" guile-boundary-e2e
FOOT_PID=""
cleanup() {
    [ -z "$FOOT_PID" ] || kill "$FOOT_PID" 2>/dev/null || true
    nested_stop
}
nested_install_cleanup cleanup
nested_start "$OUT" "${MINDE_GUILE_BOUNDARY_E2E_DISPLAY:-:99}"
seval() { timeout 15 scripts/mindectl eval "$1"; }
check() {
    result=$(seval "$2")
    [ "$result" = '#t' ] || {
        echo "error: $1: $result" >&2
        exit 1
    }
    echo "ok - $1"
}

before=$(nested_window_count)
nested_wayland_exec foot -o resize-by-cells=no -o resize-keep-grid=no sleep 300 \
    >"$OUT/foot.log" 2>&1 &
FOOT_PID=$!
nested_wait_for_window_after "$before" 15
check 'invalid geometry and batches preserve a real window' '
(let* ((id (focused-window-id))
       (original (wm-window-geometry id))
       (place (lambda args (apply rust-call (cons (quote wm-place-window) args)))))
  ;; Keep the committed size; position changes are visible synchronously.
  (unless (apply place (append (list id 50 60) (cddr original)))
    (error "valid placement failed"))
  (unless (equal? (wm-window-geometry id) (append (list 50 60) (cddr original)))
    (error "valid placement did not reach compositor"))
  (let ((before (wm-window-geometry id)))
    (for-each
     (lambda (fields)
       (unless (eq? #f (apply place (cons id fields)))
         (error "bad geometry accepted" fields)))
     (quote ((0 0 -50 300) (0 0 300 -50) (4294967306 0 300 200)
             (0 0 400.0 300) (2147483647 0 300 200))))
    (for-each
     (lambda (bad)
       (unless (eq? #f (rust-call (quote wm-place-windows)
                                  (list (list id 1 2 300 200) bad)))
         (error "bad batch accepted" bad))
       (unless (equal? before (wm-window-geometry id))
         (error "partial placement applied" bad)))
     (list (list id 0 0 (expt 2 70) 300)
           (list id 0 0 400.0 300)
           (list id 0 0 400 300 42)
           (list id 0 0 400 300 #t (quote extra))))
    (equal? before (wm-window-geometry id))))'

seval '(define boundary-key-count 0)' >/dev/null
xdotool search --name Smithay windowfocus
expected=0
for value in '#t' '(if #f #f)' '(list 1)'; do
    seval "(define (wm-handle-key . args)
             (set! boundary-key-count (+ 1 boundary-key-count)) $value)" >/dev/null
    xdotool key F5
    expected=$((expected + 1))
    attempt=0
    while [ "$(seval 'boundary-key-count')" != "$expected" ]; do
        attempt=$((attempt + 1))
        [ "$attempt" -lt 30 ] || exit 1
        sleep 0.1
    done
done
echo 'ok - real key events accept boolean, unspecified and list hook results'

check 'invalid mode values leave the output configuration unchanged' '
(let ((before (output-heads)))
  (for-each
   (lambda (mode)
     (unless (catch #t (lambda () (configure-output! "winit" #:mode mode) #f)
                       (lambda args #t))
       (error "bad mode accepted" mode))
     (unless (equal? before (output-heads))
       (error "bad mode changed outputs" mode)))
   (quote ((1280 800 -2147483648) (1280 800 -1) (1280 800 2147483648.0)
           "1280x800@-inf" "1280x800@inf" "1280x800@NaN"
           "1280x800@-0.0001" "1280x800@2147483.648")))
  #t)'

nested_wayland timeout 20 "$NESTED_PROTOCOL_CLIENT" output-mode >"$OUT/output-mode.log" 2>&1 || {
    cat "$OUT/output-mode.log" >&2
    exit 1
}
cat "$OUT/output-mode.log"

seval '(define (output-configuration-allowed?) (list 1))' >/dev/null
nested_wayland timeout 15 wlr-randr --output winit --scale 1.25 >"$OUT/output.log" 2>&1
check 'external output requests accept a true Scheme value' \
    '(= 1.25 (assq-ref (car (output-heads)) (quote scale)))'
seval '(define (output-configuration-allowed?) #f)' >/dev/null
if nested_wayland timeout 15 wlr-randr --output winit --scale 1 >>"$OUT/output.log" 2>&1; then
    echo 'error: false output policy accepted a request' >&2
    exit 1
else
    [ "$?" -eq 1 ]
fi
check 'false output policy rejects a request and keeps the desktop alive' \
    '(= 1.25 (assq-ref (car (output-heads)) (quote scale)))'

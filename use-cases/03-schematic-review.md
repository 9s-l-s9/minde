# 03 -- An A0 schematic across synchronised panes

**Request.**  "This is a 60-page A0 electrical schematic as a PDF.
Every page has a title block bottom right, a parts table on the left,
and the circuit in the middle.  I either see the whole page and read
nothing, or zoom in and lose track.  Make this reviewable."

Boundary: minde sees windows, not contents.  It cannot read a PDF page.
It can measure frames to the pixel, place windows deterministically,
type keys into chosen windows, and take screenshots.  Combined with
ordinary command-line tools that is enough.

## Session

### 1. Learn the document with ordinary tools  (outside minde)

`pdfinfo` gives 1189 x 841 mm landscape.  `pdftotext -bbox` on page 1
gives text block positions: parts table in the left 22 %, title block
in the bottom-right 15 %, circuit elsewhere.

### 2. Build the layout from the document  (exists)

```scheme
(create-group! "schematic")
(apply-layout-spec! '(hsplit 0.22 leaf (vsplit 0.82 leaf leaf)))
(dump-frames)
```

Three frames whose proportions come from the document, not a preset.

### 3. Open the file three times into known slots  (exists)

```scheme
(add-placement-rule! "schematic-parts"   #:frame 0)
(add-placement-rule! "schematic-circuit" #:frame 1)
(add-placement-rule! "schematic-title"   #:frame 2)
(spawn! "zathura --page 1 --mode fullscreen --class schematic-parts   plant.pdf")
(spawn! "zathura --page 1 --mode fullscreen --class schematic-circuit plant.pdf")
(spawn! "zathura --page 1 --mode fullscreen --class schematic-title   plant.pdf")
```

Rules are in place before the first window maps, so there is no race.

### 4. Aim each viewer at its region  (exists; plan:E1 removes the focus dance)

```scheme
(define (aim! id zoom-keys pan-keys)
  (let ((was (focused-window-id)))
    (focus-window-by-id! id)
    (for-each send-key zoom-keys)
    (for-each send-key pan-keys)
    (focus-window-by-id! was)))
```

Zoom and pan counts are computed from `(window-geometry id)` and the
bounding boxes from step 1.  The first attempt is off; the agent takes
a screenshot (`wm-screenshot` today, `(screenshot path #:frame 0)` with
plan:B1), looks at it, sends two more `l` presses.  Second screenshot
is right.  No restart, no file edit.

### 5. Make three viewers act as one document  (exists)

```scheme
(define viewers (list parts-id circuit-id title-id))
(define (all-viewers key)
  (let ((was (focused-window-id)))
    (for-each (lambda (id) (focus-window-by-id! id) (send-key key)) viewers)
    (focus-window-by-id! was)))
(register-command! 'schem-next (lambda () (all-viewers "J"))
  #:summary "Next schematic page, all panes")
(register-command! 'schem-prev (lambda () (all-viewers "K"))
  #:summary "Previous schematic page, all panes")
(bind-prefix-key! "n" (lambda () (invoke-command 'schem-next)) "schematic: next page")
(bind-prefix-key! "p" (lambda () (invoke-command 'schem-prev)) "schematic: previous page")
```

### 6. "The circuit is still too small."  (exists)

`resize-frame!` on the divider, re-read the rectangle, recompute the
zoom, send the keys.  Seconds.

### 7. Second monitor plugged in  (plan:B5 for the hook payload)

```scheme
(add-event-hook! 'output-added
  (lambda (output)
    (focus-window-by-id! parts-id)
    (move-window-to-head! (assq-ref output 'id))))   ; sketch
```

## Against a dispatcher API

Reading the PDF, taking screenshots and judging them are identical
under any compositor.  The compositor-specific part:

- exact frame geometry as data before and after every change;
- three identical programs landing in three known slots regardless of
  start order (a dynamic tiler produces an unknown tree);
- key injection into a chosen window with focus restored, three
  windows in one atomic step (`wtype`/`ydotool` type into whatever is
  focused);
- the synchronised page turn becoming a named command with help text.

## Where it stops

Three viewers on one PDF is a trick, not a feature; it breaks if the
viewer remaps keys, and the agent steers by screenshot because zoom is
not reported.  The point is that when no application does what you
need, an agent with a REPL into the compositor can build a working
approximation from ordinary programs in minutes and refine it while you
watch.

# 05 -- Circle a UI bug on the live screen

**Request.**  "That dialog is rendering wrong.  Look."

Instead of a screenshot tool, an image editor and a file upload, the
user draws on the live screen.  The compositor knows what is under the
ink, so the agent receives shapes *and* the window ids and frame they
cover, then a screenshot with the annotations baked in.

Everything here depends on PLAN.md epic H.  It is recorded because it is
the most compelling agent-facing feature discussed and should shape H's
design.

## Session

### 1. User side  (plan:H2, H4)

`C-t D` enters annotation mode.  The pointer draws a rectangle around
the broken widget; `t` adds a text label typed through `read-one-line`;
Return ends the mode.  The `annotate-report` command takes a screenshot
with the annotations, writes the sidecar, and publishes an
`annotation-report` event.

### 2. Agent side  (plan:H2, H3, B3)

The agent, subscribed with `mindectl subscribe --events`, receives:

```scheme
(annotation-report
  #:image "/home/samuel/.local/state/minde/annotations/20260905-141203.png"
  #:sidecar "/home/samuel/.local/state/minde/annotations/20260905-141203.geometry.scm"
  #:shapes ((rect (1412 388 220 64) #:colour "#ff4040"
                  #:windows (23) #:frame 1
                  #:label "checkbox misaligned")))
```

It reads the image, knows window 23 is a GTK dialog of pid 4711 whose
parent is window 7 (plan:A1), and can crop the image to the shape's
rect using the sidecar's pixel coordinates.  No guessing which window
the user meant.

### 3. Agent responds in kind  (plan:H5)

```scheme
(highlight-window! 23 #:colour "#40a0ff" #:label "resizing to min-size")
(set-float-geometry! 23 '(1200 300 640 400))
(wm-run-after 3000 clear-highlights!)
```

The user sees which window the agent touched and why.

## Against a dispatcher API

Spectacle or a screenshot-then-annotate tool produces a picture.  The
compositor is the only component that can attach window identity,
frame index and process to the ink, and the only one that can draw a
response on the live screen without a screenshot round trip.

## Design notes for epic H

- Freehand polylines need simplification before storage.
- Committed overlays are pass-through; only the grab owns the pointer.
- Shapes are logical coordinates; the sidecar gives pixels per output.
- Two overlay classes (frame labels vs annotations) so captures can hide
  one and keep the other.

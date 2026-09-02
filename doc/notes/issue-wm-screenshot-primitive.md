# Feature: native Screenshot-Primitive `(wm-screenshot path [id])`

## Motivation
Der Automatisierungs-Verify-Loop (Bewerbungs-Submits, LinkedIn-Collect) hängt heute
an externem `grim`: jeder Klick wird per Vollbild-Screenshot + Crop verifiziert.
Das ist der größte Zeitfresser im Flow und macht den IPC nicht self-contained
(AUTOMATION-WISHLIST #7). Die Capture-Maschinerie existiert bereits zweifach
(`ext-image-copy-capture-v1`, `wlr-screencopy`) — eine IPC-Primitive kann sie
direkt wiederverwenden.

## Design
- `(wm-screenshot "/abs/pfad.png")` → Vollbild des Outputs unter dem Pointer.
- `(wm-screenshot "/abs/pfad.png" id)` → nur die Region des Fensters `id`
  (Geometrie aus dem `WINDOW_GEOMETRIES`-Snapshot).
- Capture ist ein **Deferred-Vorgang** (nächster Composite-Turn) → gleiche
  Token-Mechanik wie `wm-drop-files`: Primitive liefert sofort einen Token,
  Abschluss via `(wm-automation-status token)` → `(screenshot done)` /
  `(screenshot failed REASON)` plus `automation-result`-Eventzeile.
- Render-Pfad: wie `satisfy_output_captures` (`src/handlers/screencopy.rs:448`)
  über `OutputDamageTracker` + `ExportMem` in einen Offscreen-Buffer, dann als
  PNG nach `path` schreiben.

## Akzeptanz
- `(wm-screenshot "/tmp/s.png")` erzeugt binnen <200 ms ein PNG des aktiven
  Outputs inkl. Cursor-los gerenderter Fenster; Status endet auf `done`.
- Fenster-Variante liefert exakt die Fenstergeometrie (Crop-frei verwendbar).
- `tests/screencapture-e2e.sh` um den IPC-Weg erweitert (bisher nur grim).

## Verwandt
- AUTOMATION-WISHLIST #7 (native Screenshot-Primitive), #8/#9 (inzwischen
  umgesetzt: `wm-pointer-position`, `wm-window-geometry`).

## Status: IMPLEMENTIERT (2026-08-31)
`(wm-screenshot path [window-id])` liefert Token; Abschluss via
`(wm-automation-status token)` → `(screenshot done|failed)` + Eventzeile.
Render über die bestehende Capture-Queue (`CaptureFrame::File`,
`render_to_png` in `src/handlers/screencopy.rs`), PNG-Writer dependency-frei
(`src/png.rs`, stored-deflate). Cursor wird nicht mitgerendert.
Headless-Smoke bestanden: 1280×800-PNG, `identify` parst es, Status `done`.
Offen: `tests/screencapture-e2e.sh` um den IPC-Weg erweitern.

## Real-World-Verifikation (2026-09-01, nested Session + Zen + Ashby)
Nested `run-nested`-Session (neues Binary), frisches Zen-Profil, echte Seiten:
20 Screenshots im Live-Flow als Verify-Loop genutzt (Output-Variante, ~1s
Turnaround inkl. Deferred-Status). Fenster-Variante bisher nur headless
getestet. Offen bleibt tests/screencapture-e2e.sh (IPC-Weg).

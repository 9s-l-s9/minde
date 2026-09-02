# Feature: synthetische Drag-&-Drop-Primitive `(wm-drop-files …)`

## Zusammenfassung
minde soll einen **compositor-initiierten Drag-&-Drop** synthetisieren können, der
eine Menge Dateien (als `text/uri-list`) auf die Surface an einer Zielposition
„fallen lässt". Das ist eine generelle Wayland-Interaktion (`wl_data_device`), die
minde ohnehin sauber beherrschen sollte, und löst als Nebeneffekt das
Datei-Upload-Problem für jede „drag and drop here"-Zone (Browser-Formulare u. a.)
ohne den GTK-Portal-Datei-Dialog.

## Motivation / Use Cases (nicht nur Automatisierung)
- **Agent-/Scripting-Steuerung:** Formular-Upload-Zonen befüllen, ohne den
  synthetisch schwer bedienbaren Portal-Datei-Dialog (Enter/Doppelklick/Header-
  Button unzuverlässig, siehe AUTOMATION-WISHLIST #3/#5).
- **DnD-Tests:** eigene und fremde Clients auf Drop-Handling testen (Reorder,
  Datei-Import, Text-Drop).
- **Accessibility:** Drag-Operationen ohne physische Maus auslösen.
- **Allgemeine Workflows:** „diese Datei in Fenster X ziehen" als IPC-Einzeiler.

## Vorgeschlagene API (Guile-Gsubr)
```scheme
;; Dateien als text/uri-list auf die Surface an Output-Koordinaten (X Y) droppen.
;; PATHS ist eine Liste absoluter Pfade. Rückgabe #t bei Erfolg.
(wm-drop-files x y paths)          ; z. B. (wm-drop-files 1100 660 '("/home/…/cv.pdf"))

;; Optional, generalisiert (gleiche Mechanik, anderer MIME-Typ):
(wm-drop-text  x y text)           ; bietet text/plain;charset=utf-8 an
```
Perspektivisch könnte eine allgemeine Form die MIME-Aushandlung offenlegen:
`(wm-drop x y '(("text/uri-list" . "file:///…\r\n") ("text/plain" . "…")))`.

## Verhalten / Semantik
1. Pfade → `file://`-URIs, **percent-encoded**, mit **CRLF** verbunden und
   **abschließendem CRLF** (per `text/uri-list`-Spec, RFC 2483).
2. Pointer nach `(x, y)` warpen (Zielsurface erhält `enter`).
3. **Server-initiierten DnD-Grab** starten, mit minde als *data source*
   (angebotene MIME-Typen: mind. `text/uri-list`).
4. Kleine Pointer-Motion, damit das Ziel `wl_data_device.motion` bekommt und ggf.
   `accept`/`set_actions` sendet.
5. **Drop** auslösen; wenn das Ziel via `wl_data_offer.receive` `text/uri-list`
   anfordert, den URI-String in den fd schreiben.

## Umsetzungs-Hinweise (Smithay)
- Smithay unterstützt **server-side data sources**: Server-initiierten DnD über
  `smithay::wayland::selection::data_device::start_dnd(dh, seat, state, serial,
  start_data, icon, metadata)` starten. Der `ServerDndGrabHandler`/`DataDeviceHandler`
  liefert die Bytes: bei `send(mime_type, fd)` den URI-String schreiben.
- `start_data: PointerGrabStartData` muss synthetisiert werden (focus = Zielsurface
  unter dem Pointer, `button` = ein plausibler Code, `location` = aktuelle Pointer-
  Position). Da kein echter Button gedrückt ist, den Grab compositor-seitig
  aufsetzen (analog zu MoveSurfaceGrab in `src/grabs`).
- `metadata: SourceMetadata { mime_types: vec!["text/uri-list".into(), …], dnd_action }`.
- `icon`: optional; `None` oder eine 1×1-Cursor-Surface reicht.
- Ablauf im Event-Loop takten: enter → (motion) → drop brauchen ggf. je einen
  Loop-Turn, damit der Ziel-Client (Browser, dessen JS-`drop`-Handler) verarbeitet.
  Ein `wm-run-after-ms`-artiges Sequencing oder interne kleine Delays einplanen.

## Offene Fragen / Scope
- **XWayland:** native Wayland-Surfaces zuerst. XWayland nutzt XDND (eigene Bridge)
  — synthetischer Wayland-DnD landet dort nicht automatisch; separat tracken. (Für
  den Ashby-Fall lief Zen *nativ Wayland*, also im Scope.)
- **Button-Held-Erwartung:** Manche Clients erwarten einen gehaltenen Pointer-Button
  während des Drags. Prüfen, ob Smithays server-initiierter Grab das für Firefox/Zen
  korrekt abbildet (dort funktioniert normaler File-DnD über `text/uri-list`).
- **Aktionen:** `copy` als Default-`dnd_action` (Upload = Kopie), nicht `move`.
- **Mehrere Dateien:** von Anfang an unterstützen (Liste → mehrzeilige uri-list).

## Akzeptanzkriterien
- `(wm-drop-files X Y '("/tmp/a.pdf"))` auf eine Firefox/Zen-Dropzone
  (`<input type=file>` bzw. „drag and drop here") hängt die Datei an.
- Mehrere Pfade → mehrere Dateien angehängt.
- Manueller (echter) Drag-&-Drop funktioniert unverändert weiter.
- Rückgabe `#f` bei ungültigen Pfaden / keiner Zielsurface an (x,y).

## Testplan
- Minimale HTML-Testseite mit `<input type=file>` + Dropzone lokal öffnen, drop
  auslösen, Dateiliste prüfen.
- Realtest: eine Ashby-Bewerbungs-Dropzone (Resume-Feld) — heute manuell über den
  Portal-Dialog gelöst; mit `wm-drop-files` soll es dialoglos gehen.

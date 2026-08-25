# Bug: `wm-drop-files` wird von Browser-Dropzones abgelehnt (`drop-files rejected`)

## Symptom
`(wm-drop-files x y (list "/abs/pfad.pdf"))` liefert einen Token, und
`(wm-automation-status token)` meldet danach `(drop-files rejected)`. Die Datei
landet **nicht** in der Ziel-Dropzone.

**Reproduktion (2026-08-25, echtes Zen nativ Wayland, Ashby-Upload-Feld
„or drag and drop here"):**
- An mehreren Koordinaten innerhalb der Dropzone getestet (`1600,730`, `1665,698`,
  `1575,698`, `1500,730`) → jedes Mal `(drop-files rejected)`.
- Pfad war gültig und existent (ungültiger Pfad liefert korrekt sofort `#f`).

## Vermutete Ursache
Der DnD-Handshake ist unvollständig. Browser-Dropzones akzeptieren einen Drop nur,
wenn zuvor ein `dragover`-Event lief, dessen JS-Handler `preventDefault()` aufruft
(sonst ist der Drop nicht erlaubt). Der synthetische Ablauf enter → (kurze motion) →
drop ist vermutlich zu schnell / hat zu wenige `motion`-Frames, sodass der
`dragenter`/`dragover`-Handler des Clients nicht rechtzeitig `accept`/`preventDefault`
setzen kann → der Drop wird abgelehnt.

## Fix-Ideen
- Zwischen `enter` und `drop` **mehrere `motion`-Frames mit kurzer Verzögerung**
  einschieben (dem Client Event-Loop-Turns geben, um `wl_data_offer.accept` /
  `set_actions` zu setzen), bevor der Drop feuert.
- Optional **konfigurierbares Dwell** (`(wm-drop-files x y paths [dwell-ms])`).
- Ggf. auf das `accept` des Ziels **warten**, bevor `drop` gesendet wird (statt fixem
  Delay) — und erst dann als Erfolg werten.

## Workaround (heute genutzt)
Fallback über den GTK-Datei-Dialog: „Upload File"-Button klicken → im Dialog
Ctrl+L → Zeile leeren → Pfad per `wm-paste` → **„Open"-Button mit `(wm-click 'left)`**
klicken (der Clamp-Fix war hier der entscheidende Baustein; Enter/Return schließt den
Dialog weiterhin nicht zuverlässig). Funktionierte einwandfrei.

## Akzeptanz
`(wm-drop-files x y '("/abs/cv.pdf"))` auf eine Firefox/Zen-Dropzone
(`<input type=file>` bzw. „drag and drop here") hängt die Datei an; Status endet auf
`done`/`accepted` statt `rejected`. Mehrere Pfade → mehrere Dateien.

## Verwandt
- `issue-wm-type-drops-modifier-chars.md` (separates Finding derselben Session)
- Ursprüngliches Feature-Issue: `issue-wm-drop-files.md`

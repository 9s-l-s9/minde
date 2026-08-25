# minde — Automation/Scripting Wishlist

Gesammelt beim Fahren von echten Browser-Formularen (Ashby-Bewerbungen) über den
IPC-Socket `$XDG_RUNTIME_DIR/minde-ipc.sock` (2026-08-25). Ziel: minde als
zuverlässiges, headless-artiges Automatisierungs-Target. Sortiert nach Reibung.

## 🐞 Bugs / Footguns (haben real Zeit gekostet)

1. **`wm-click` clampt Button auf 1..3 — 272/BTN_LEFT wird zu Rechtsklick.**
   `let button = to_i64(button).clamp(1, 3)` (src/guile/mod.rs ~485). Wer intuitiv
   den evdev-Code `0x110` (272, BTN_LEFT) übergibt, bekommt `3` = Rechtsklick →
   ständig Kontextmenüs. Die Doku sagt nur „Synthesize a pointer BUTTON press and
   release", ohne die 1/2/3-Konvention.
   **Fix-Optionen:** (a) Docstring: „button: 1=left, 2=middle, 3=right"; (b) evdev-
   Codes `0x110/0x111/0x112` zusätzlich akzeptieren; (c) Symbole erlauben
   `(wm-click 'left)`. Mindestens (a).

2. **`wm-send-string` verliert führende Zeichen.**
   „Samuel Levi Schmidt" kam als „ Schmidt" an (nur der Rest nach der letzten
   Pause). Vermutlich feuert die virtuelle Tastatur schneller als der Client die
   Fokus-/Repeat-Events verarbeitet. Aktuell unbrauchbar für Formularfelder — wir
   mussten auf `wm-set-clipboard` + Ctrl+V ausweichen.
   **Fix:** konfigurierbares Inter-Key-Delay, oder pro Zeichen auf ein
   commit/frame warten.

3. **`wm-send-key … "Return"` aktiviert GTK-Dialoge nicht.**
   Im GTK-Portal-Datei-Dialog (File-Upload) reichte Ctrl+V an (Pfad erschien),
   aber „Return"/Enter löste das Öffnen nie aus — weder in der Location-Zeile noch
   bei selektierter Datei. `wm-send-key 4 "a"`/`4 "v"`/„Tab"/„Escape"/„F12" gingen.
   Verdacht: Keysym-Handling für Return, oder Keyboard-Focus-Routing in modale
   Child-Surfaces. **Bitte prüfen:** erreicht `wm-send-key` override-redirect/modale
   Fenster zuverlässig, und ist „Return" das korrekte Keysym?

## ✨ Feature Requests (würden Automatisierung enorm glätten)

4. **Synthetische Drag-&-Drop-Primitive `(wm-drop-files x y paths)`.**
   Löst den Datei-Upload dialoglos (jede „drag and drop here"-Zone) und ist eine
   generelle `wl_data_device`-Interaktion. Ausführliches Issue: `issue-wm-drop-files.md`.

5. **Scroll-Primitive.** Es gibt keinen Weg, ein Achsen-/Scroll-Event zu senden
   (`zwlr_virtual_pointer` axis). Workaround: Feld blurren + `Page_Down` — bricht,
   wenn ein Textfeld fokussiert ist. Wunsch: `(wm-scroll dx dy)` bzw.
   `(wm-axis horizontal vertical)`.

6. **Doppelklick.** Zwei schnelle `wm-click`-Aufrufe wurden von GTK NICHT als
   Doppelklick erkannt (Datei in Liste öffnen). Wunsch: `(wm-double-click button)`
   oder `(wm-click button count)` mit korrekten Klick-Zeitstempeln/Serials.

7. **Native Screenshot-Primitive.** Wir nutzen extern `grim`. `(wm-screenshot path
   [id|region])` würde den IPC self-contained machen (Vollbild / Fenster / Region).

8. **Pointer-Position lesen.** `(wm-pointer-position)` → `(x y)`. Zum Verifizieren
   von `wm-warp-pointer` und Debuggen von Koordinaten. Aktuell blind.

9. **Native Fenster-Geometrie in Output-Koordinaten.** `(window-geometry id)` →
   `(x y w h)` im selben Koordinatensystem wie `wm-warp-pointer`. Dann können
   Skripte Feld-Koordinaten relativ zum Fenster rechnen statt absolute Bildschirm-
   Pixel zu raten. (Aktuell nur via xdotool-XWayland-Umweg, nicht für native
   Wayland-Fenster.) Bitte auch dokumentieren, ob `wm-warp-pointer`-Koordinaten die
   Topbar/Gaps einschließen (0-basiert vs. usable-rect).

10. **Zuverlässiges Paste/Type high-level.** Da (2) bricht, wäre `(wm-type "text")`
    (intern via Clipboard+Paste ODER sauber getaktete Keys) die nützlichste einzelne
    Automatisierungs-Primitive. Dazu `(wm-paste)` = „füge Clipboard in fokussierte
    Surface ein" (Äquiv. Ctrl+V), unabhängig vom App-Keymap.

11. **Clipboard vs. Primary-Selection klarstellen.** `wm-set-clipboard` heißt „Set
    the primary clipboard contents" — mehrdeutig. `wm-request-paste` fügt die
    PRIMARY-Selection ein, Ctrl+V die CLIPBOARD — die desyncen (ein Ctrl+A
    überschreibt Primary mit der Selektion). Wunsch: getrennt `(wm-set-clipboard)`
    / `(wm-set-primary)` + klare Doku, welche Selection welche Funktion betrifft.

## 🧭 Fokus/Fenster (kleinere Klarstellungen)

12. **`wm-focus-window`/`wm-raise-window` holen ein Fenster im Tiling-Layout NICHT
    nach vorn** (Konsole blieb oben). Nur `focus-window-by-id!` (Frame/Gruppen-
    Wechsel) brachte das Ziel sichtbar nach vorn. Entweder Doku-Hinweis, oder
    `wm-raise-window` soll über Frames/Gruppen hinweg tatsächlich sichtbar raisen.
    Bei Child-/Dialog-Fenstern (File-Upload) war Fokussieren zusätzlich zickig.

## 🔎 Neue Findings (2026-08-25, nach dem großen Update — HiPeople erfolgreich gefahren)

Das Update hat fast die ganze Liste umgesetzt (danke!). Zwei Rest-Punkte tauchten beim
echten Fahren auf:

A. **`wm-type` droppt Zeichen, die einen Modifier brauchen.** „samuel@schmidt-contact.com"
   kam als „samuelschmidt-contact.com" an (`@` fehlte), „https://…" als „https//…" (`:`
   fehlte). Plain-Buchstaben + `/` gehen, aber Shift-/AltGr-Symbole (`@ : ! ? …`) fallen
   raus — vermutlich wird der nötige Modifier nicht mitsynthetisiert, oder die Keysym→
   Keycode-Abbildung ignoriert Level-2/3-Symbole. **Workaround:** für Sonderzeichen
   `wm-set-clipboard` + `wm-paste` (layout-unabhängig, 100 % zuverlässig). **Fix-Idee:**
   `wm-type` intern über die Clipboard+Paste-Route laufen lassen, ODER pro Zeichen den
   passenden Modifier-Level aus der xkb-Keymap auflösen.

B. **`wm-drop-files` wird von Browser-Dropzones abgelehnt** (`(drop-files rejected)`),
   an mehreren Koordinaten getestet. Vermutlich der DnD-Handshake: der Browser akzeptiert
   den Drop nur, wenn zuvor ein `dragover`-Event mit `preventDefault()` lief — der
   synthetische Ablauf enter→motion→drop ist zu schnell/unvollständig, sodass der
   JS-`dragover`-Handler nicht greift. **Fix-Idee:** zwischen enter und drop ein paar
   `motion`-Frames mit kurzer Verzögerung einschieben (dem Client Zeit geben,
   `accept`/`preventDefault` zu setzen), evtl. konfigurierbares Dwell. **Fallback heute:**
   Datei-Dialog + gefixter `wm-click 'left` auf „Open" — funktionierte einwandfrei
   (der Clamp-Fix war genau der fehlende Baustein).

## Kontext / was schon super funktioniert
- IPC-Socket + `(ok RESULT)`-Envelope: sehr angenehm zum Scripten.
- `all-window-ids` / `window-title` / `window-app-id`: perfekt zum Fenster-Finden.
- `wm-set-clipboard` + `wm-send-key 4 "v"`: der zuverlässige Text-Eingabe-Weg.
- **Der entscheidende Win:** nativer Compositor-Input hat KEIN „remote control"-
  Flag → reCAPTCHA v3 / Ashby lassen durch, wo CDP/Marionette/Playwright blocken.
  minde ist damit ein besseres Browser-Automatisierungs-Target als jedes
  WebDriver-Framework. Die obigen Fixes machen es rund.

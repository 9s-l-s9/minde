# Playbook: echte ATS-/Web-Formulare mit minde fahren

Erfahrungswissen aus dem Fahren echter Bewerbungsformulare (Ashby, Personio,
SmartRecruiters, Lever, Gem, Workday) über den minde-IPC-Socket. **Das sind
App-/Browser-seitige Macken, keine minde-Bugs** — sie kosten trotzdem Zeit, wenn man
sie nicht kennt. minde-eigene Bugs/Wünsche stehen in `AUTOMATION-WISHLIST.md` und den
`issue-*.md`-Dateien.

Stand: 2026-08-29 (nach der atira-Ashby-Bewerbung).

## Goldene Regeln (haben sich durchgesetzt)

1. **Ein Feld pro IPC-Aufruf, dann verifizieren.** Mehrere Textfelder in schneller
   Folge in einem Aufruf zu füllen führt bei Ashby/SmartRecruiters/Gem/Personio zum
   **Field-Shift-Bug**: jeder Wert landet ein Feld „später". Zuverlässig: pro Feld ein
   eigener Aufruf `wm-set-clipboard` → warpen → klicken → `Ctrl+A` → `Delete` →
   `Ctrl+V`.

2. **Paste hat ~1 s Render-Lag.** Nach `Ctrl+V` erscheint der Text erst ~1 s später.
   → **ein** Paste, dann ≥1,5–2 s warten, DANN screenshotten/verifizieren. Nie auf
   Basis eines Sofort-Screenshots erneut pasten — sonst **verdoppelter Inhalt**.

3. **Sonderzeichen immer per Clipboard+Paste, nicht `wm-type`.** `wm-type` droppt
   Modifier-Symbole wie `@` und `:` (E-Mails, URLs). `+`, Leerzeichen und Ziffern gehen
   inzwischen (2026-08-29). Für alles mit `@ : ! ?` → Clipboard.

4. **Scrollen über Tab-Walk.** `wm-scroll` wirkt nicht; `Page_Down`/`End` scrollen
   nicht bei fokussiertem Feld. Verlässlich: Feld fokussieren → mehrfach `Tab` — der
   Browser scrollt das fokussierte Element in den Sichtbereich, bis Submit erreicht ist.
   (Details: `issue-wm-scroll-no-effect.md`.)

5. **Datei-Upload über den GTK-Dialog, nicht Drag&Drop.** `wm-drop-files` wird von
   Browser-Dropzones abgelehnt (`issue-wm-drop-files-rejected-by-dropzones.md`). Statt-
   dessen: „Upload File"-Button klicken → im GTK-Dialog `Ctrl+L` → Pfad per Clipboard
   einfügen → **„Open"-Button klicken** (Enter/Return schließt den Dialog nicht
   zuverlässig).

## Feld-Typ-Macken (ATS-übergreifend)

- **Telefon-Feld ist oft ein HTML-`<input type=number>`** (erkennbar am ↕-Spinner
  rechts). Es **verwirft `+` und Leerzeichen** und **löscht manuelle Eingaben bei jedem
  Blur** wieder — nur reine Ziffern persistieren. Und: „Autofill from resume" füllte es
  NICHT. → **reine Ziffern tippen**, z. B. `491625697085` (= +49 162 5697085 ohne `+`/
  Leerzeichen; Führungs-0 wird gestrippt, also Ländercode ohne 0 voranstellen).

- **Radios & Yes/No-Segmented-Toggles: den LABEL-TEXT klicken, nicht den Kreis/die
  Pill-Mitte.** Der Klick auf das Grafik-Element registriert oft gar nicht. Klick auf
  den Text („Yes", „Social media …") selektiert. Selektions-Anzeige ist subtil
  (schwarz gefüllte Pill bzw. blauer Radio-Dot) → per Crop-Screenshot verifizieren.
  Klicks sind flaky: warpen → ~400 ms → klicken → ~300 ms drauf bleiben → verifizieren.

- **Datepicker (Ashby):** Feld anklicken öffnet Kalender; Monats-Pfeile (`→`) mehrfach
  klicken (registrieren mit demselben ~1 s Verzug wie Paste — nicht vorschnell
  re-klicken); dann Tag anklicken. Ergebnis erscheint als `MM/DD/YYYY` im Feld.

- **„Autofill from resume" (Ashby):** parst Name/E-Mail/LinkedIn/GitHub aus dem
  CV-PDF und setzt sie als committed React-State (blur-fest). **Telefon parst es nicht.**
  Überschreibt vorhandene Werte nicht destruktiv, wenn sie korrekt sind. Nützlich, aber
  Pflichtfelder danach trotzdem einzeln prüfen.

## Ashby-spezifisch
- Kein Login/Konto nötig (self-service). Wird — anders als Playwright/headless — nicht
  als Automation geflaggt, weil minde echte Eingabe-Events in den echten Zen schickt.
- Reihenfolge im Formular ist lang: Name, E-Mail, Phone, LinkedIn, GitHub, Resume,
  Cover Letter (optional), dann custom Fragen (Start-Datum, Standort-Ja/Nein, „years of
  experience", „how did you hear", …), dann Submit. Submit meldet fehlende Pflichtfelder
  rot mit Sprung-Link — den Link klicken, um zum Feld zu scrollen.
- **JD ist JS-only**: `fetch-posting.py`/WebFetch scheitern (leerer SPA-Shell). Nicht mit
  gh-API o. Ä. umwegen — **direkt in Zen laden und per Screenshot lesen** (man ist zum
  Bewerben eh dort). Der „Application"-Tab enthält das Formular.
- **Ashby-Datepicker:** Monats-Pfeile schwer per Pixel zu treffen. **Direkt „MM/DD/YYYY"
  ins Feld tippen funktioniert** (z. B. „12/01/2026" → Kalender springt auf den Monat,
  Tag wird markiert); danach optional den Tag klicken zum Commit.
- **Location-Autocomplete:** Klick auf den Vorschlag setzt einen Chip; einen zweiten Ort
  anhängen ist zickig (Suchtext mischt sich mit dem Chip) — ein sauberer Ort genügt.

## Zen (Firefox-Base) aus der Bash-Shell (neu) starten
Wenn Zen geschlossen ist und aus der Claude-Bash-Shell gestartet werden muss:
- **Nackt gestartet crasht Zen** (`Gtk:ERROR … image-missing.png … Unrecognized image file
  format` → `mozalloc_abort` → Segfault), weil die GTK/pixbuf-Env der Shell fehlt.
- **Fix:** mit explizit gesetzten Guix-Env-Variablen starten (Pfade aus
  `~/.guix-home/profile`):
  ```
  P=/home/samuel/.guix-home/profile
  GDK_PIXBUF_MODULE_FILE=$P/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache \
  GIO_MODULE_DIR=$P/lib/gio/modules GTK_PATH=$P/lib/gtk-3.0 \
  FONTCONFIG_FILE=$P/etc/fonts/fonts.conf MOZ_ENABLE_WAYLAND=1 \
  bash -lc 'exec /home/samuel/.guix-home/profile/bin/zen'
  ```
  Als **run_in_background-Task** starten (bleibt am Leben). `setsid` fehlt im PATH.
- Neues Zen-Fenster erscheint mit neuer ID (`(all-window-ids)`), app-id „zen".

## Fokus-Konflikt Konsole ↔ Zen (WICHTIG)
Die Claude-Bash-Kommandos laufen in der **Konsole** (eigenes Fenster/Gruppe), die sich
nach jedem Kommando in den Vordergrund holt. minde-Key-Events gehen dann in die Konsole
statt in Zen (Symptom: URL navigiert nicht, Screenshot zeigt die Konsole).
**Fix:** vor JEDEM Key-/Klick-Batch `(focus-window-by-id! <zen-id>)` (ggf. zusätzlich
`(switch-to-group! " II ")`) voranstellen, und im selben Bash-Aufruf gebündelt agieren.

## Playwright-MCP: warum NICHT der Default
Playwright startet einen **eigenen, automatisierten Browser** → viele Portale (v. a.
Ashby) erkennen und blocken das. An Samuels echten Zen (Firefox-basiert) lässt sich
Playwright praktisch nicht andocken (kein unterstütztes Attach an laufendes Firefox).
Deshalb bleibt **minde (echte Events in echten Zen)** das primäre Automatisierungs-
Werkzeug; die Verbesserungen gehören in minde (Scroll-Primitive, Klick-Zuverlässigkeit,
DnD-Handshake), nicht in einen zweiten Browser-Stack.

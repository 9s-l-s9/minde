# Bug: `wm-type` / `wm-send-string` droppen Zeichen, die einen Modifier brauchen

## Symptom
Beim Tippen von Formularfeldern über `(wm-type "…")` fallen alle Zeichen weg, die
einen Shift-/AltGr-Level brauchen — Buchstaben und `/` gehen, `@ :` (und vermutlich
`! ? € {} …`) nicht.

**Reproduktion (2026-08-25, echtes Zen, Ashby-Formular):**
- `(wm-type "samuel@schmidt-contact.com")` → Feld enthielt `samuelschmidt-contact.com`
  (das `@` fehlte).
- `(wm-type "https://www.linkedin.com/in/…")` → `https//www.linkedin.com/in/…`
  (der `:` fehlte; `/` kam durch).

## Vermutete Ursache
Die paced synthetische Tastatur löst nur Level-0-Keysyms aus. Zeichen auf Level 2/3
der aktiven xkb-Keymap (Shift bzw. AltGr) werden ohne den nötigen Modifier gesendet
→ der Client bekommt ein anderes/kein Zeichen. Betrifft `wm-type` und `wm-send-string`.

## Fix-Ideen
1. **Einfach & robust:** `wm-type` intern über die Clipboard-Route laufen lassen
   (`set CLIPBOARD` → synthetisches Ctrl+V), layout-unabhängig. Ggf. als Default,
   mit optionalem „echtes Tippen"-Flag für Fälle, wo Keystroke-Events nötig sind.
2. **Korrekt:** pro Zeichen den passenden Keysym→(keycode, modifier-level) aus der
   xkb-Keymap auflösen und den Modifier (Shift/ISO_Level3_Shift) mit drücken/lösen.

## Workaround (heute genutzt)
Für Felder mit Sonderzeichen: `(wm-set-clipboard "…")` + Feld anklicken +
`(wm-send-key 4 "a")` (Ctrl+A) + `(wm-paste)`. 100 % zuverlässig, keymap-unabhängig.

## Akzeptanz
`(wm-type "a@b:c/d!e?f")` landet zeichengenau im fokussierten Feld — inklusive
Shift-/AltGr-Symbolen, unabhängig vom aktiven Tastaturlayout.

## Status: FIX IMPLEMENTIERT (2026-08-31)
Hybrid aus Fix-Idee 1+2: Zeichen, die die Keymap auflösen kann, werden weiter
echt getippt (inkl. Modifier); **nicht auflösbare Zeichen (z. B. AltGr-Symbole
ohne realen Modifier-Key) fallen pro Zeichen auf Clipboard+Ctrl+V zurück** —
der Clipboard-Write läuft als queued SyntheticAction in Reihenfolge, nichts
wird mehr gedroppt. Umsetzung in `src/state.rs` (`send_string`,
`SyntheticAction::SetClipboard`). Achtung: das Clipboard enthält danach das
letzte Fallback-Zeichen.

## Real-World-Verifikation (2026-09-01, nested Session + Zen + Ashby)
Nested `run-nested`-Session (neues Binary), frisches Zen-Profil, echte Seiten:
Zen-URL-Leiste: `(wm-type "a@b:c_x?!")` kommt vollständig und in richtiger
Reihenfolge an (@ : ? ! vorher gedroppt). Clipboard-Fallback bestätigt.
**VERIFIZIERT, kann zu.**

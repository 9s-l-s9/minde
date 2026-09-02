# Bug/Feature: Klicks auf custom Controls flaky, Doppelklick unerkannt, Paste-Lag — Primitive sollen „settlen"

## Symptom
Drei Timing-Races aus echten Ashby-/GTK-Sessions (AUTOMATION-WISHLIST E/F, #6):

1. **Klick auf custom React-Controls (Radio/Segmented-Pill) registriert oft nicht.**
   Muster, das schließlich klappte: warpen, ~400 ms warten, klicken, ~300 ms
   draufbleiben, per Screenshot verifizieren. Verdacht: der synthetische Klick
   feuert im selben Turn wie die Warp-Motion — der Client hat enter/motion noch
   nicht verarbeitet, Hover-State/Hit-Target steht noch nicht.
2. **Zwei schnelle `wm-click` werden von GTK nicht als Doppelklick erkannt**
   (Datei in Liste öffnen ging nicht).
3. **Paste rendert ~1 s verzögert** — wer sofort screenshottet, hält das Feld für
   leer und pastet doppelt.

## Fix-Ideen
1. **Hover→settle→click in `wm-click` einbauen:** vor dem Button-Press eine echte
   motion+frame an der aktuellen Position senden und dem Client 1–2 Event-Loop-
   Turns geben (konfigurierbares `settle_ms`, Default ~150–300 ms), Press dann
   ~100 ms halten. Macht den heutigen Hand-Workaround zum Default-Verhalten.
2. **`(wm-click button count)`** mit korrektem Multi-Klick-Timing (Abstand deutlich
   unter der Client-Doubleclick-Schwelle, gleiche Koordinate, saubere Serials).
3. **Paste-Settle dokumentieren/verbessern:** `wm-paste` kann den Client-Commit
   nicht abwarten (synchroner IPC-Reply-Kontrakt) — aber die Kombination
   `wm-paste` + natives `wm-screenshot` (separates Issue) macht den
   Verify-Zyklus billig genug, um die 1,5–2 s-Regel einzuhalten. Optional:
   `(wm-paste settle-ms)`, das den Reply um settle-ms verzögert (paced queue),
   damit Skripte nicht selbst schlafen müssen.

## Akzeptanz
- 10/10 Klicks auf einen Ashby-Radio-Label treffen ohne manuelles Warten.
- `(wm-click 'left 2)` öffnet eine Datei in einer GTK-Liste.
- Nach `(wm-paste 2000)` zeigt der direkt folgende Screenshot den Feldinhalt.

## Verwandt
- AUTOMATION-WISHLIST E (Klick-Flakiness), F (Paste-Lag), #6 (Doppelklick).
- `issue-wm-screenshot-primitive.md` (billige Verifikation).

## Status: TEILWEISE IMPLEMENTIERT (2026-08-31)
Fix 1+2 umgesetzt: `wm-click` sendet jetzt vor dem Press eine **Hover-Motion +
150 ms Settle**, hält den Button 40 ms und nutzt 80 ms Multi-Klick-Abstand
(`SyntheticAction::Hover` in `src/state.rs`) — der manuelle
warp→warten→klicken-Workaround ist damit Default. `(wm-click 'left 2)` existierte
bereits (count-Parameter) und profitiert vom neuen Timing. Fix 3 (Paste-Settle)
bewusst nicht umgesetzt: synchroner IPC-Reply-Kontrakt; stattdessen billige
Verifikation via neuem `wm-screenshot`. Ashby-Real-Test steht aus.

## Real-World-Verifikation (2026-09-01, nested Session + Zen + Ashby)
Nested `run-nested`-Session (neues Binary), frisches Zen-Profil, echte Seiten:
Zen-Onboarding (3 Buttons) und echte Ashby-Radios (direkt auf den Kreis, kein
Label-Umweg nötig): jeder Klick sitzt beim ersten Versuch. Hover→Settle→Click
bestätigt. **VERIFIZIERT.**

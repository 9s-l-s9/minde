# Bug: `wm-scroll` bewegt Web-Content nicht (kein Scroll-Effekt in Firefox/Zen)

## Symptom
`(wm-scroll dx dy)` löst in Firefox/Zen **keine sichtbare Scroll-Bewegung** aus —
weder auf einer inneren Scroll-Liste (LinkedIn Jobs) noch auf einer **ganz normalen
langen Dokument-Seite**.

**Reproduktion (2026-08-25):**
- Wikipedia „Linux"-Artikel in Zen geöffnet, Pointer per `(wm-warp-pointer 960 600)`
  über den Content (mit `(wm-pointer-position)` als `(960 600)` verifiziert).
- `(wm-scroll 0 4000)` 6× in Folge → Vorher/Nachher-Screenshot **identisch**, Seite
  steht unverändert am Artikelanfang.
- Auch große Werte (`(wm-scroll 0 10000)` 20×) und die LinkedIn-Ergebnisliste:
  keinerlei Bewegung, DOM-/Sichtbereich unverändert.
- Gegenprobe: **Tastatur-Scroll funktioniert** — nach Klick in den Body scrollt
  `(wm-send-key 0 "Page_Down")` die Seite normal weiter. Also liegt es an `wm-scroll`,
  nicht am Fokus/der Seite.

## Vermutete Ursache
`wm-scroll` sendet laut Doku „a continuous pointer-axis frame". Firefox/Zen reagiert
auf **kontinuierliche (high-res) Achsen-Frames** ohne die klassischen Wheel-Marker
nicht als Scroll. Wahrscheinlich fehlt:
- `wl_pointer.axis_discrete` bzw. **`axis_value120`** (die diskrete Wheel-„Klick"-
  Menge, die Firefox erwartet), und/oder
- ein `axis_source` (WHEEL vs. FINGER) plus ein sauberer `wl_pointer.frame`-Abschluss
  (und ggf. `axis_stop`).

## Fix-Ideen
- `wm-scroll` zusätzlich `axis_value120` (bzw. `axis_discrete`) mit `axis_source =
  wheel` senden, in einem vollständigen `frame`. Ein „Notch" = 120.
- Einheiten klären/dokumentieren: `dy` als **Notches** interpretieren (1 = ein
  Mausrad-Klick) statt roher wl_fixed-Werte, dann skaliert es intuitiv.
- Test: Wikipedia-Langseite muss mit `(wm-scroll 0 3)` ~3 Wheel-Klicks weit scrollen.

## Workaround (heute genutzt)
- Normale Dokument-Seiten: `(wm-send-key 0 "Page_Down")` / `"space"` nach Klick in
  den Body — scrollt zuverlässig.
- LinkedIn-Ergebnisliste (innerer Scroll-Container): **kein Scroll nötig**, da `Ctrl+A`
  ohnehin das gesamte DOM (alle ~25 Jobs der Seite) selektiert; weitere Treffer via
  `&start=N`-URL.

## Nachtrag 2026-08-29 (langes Ashby-Formular): Tab-Walk als zuverlässiger Scroll
`wm-scroll` scrollte weiterhin nicht. Zusätzliches Problem: **`Page_Down`/`Page_Up`/
`End`/`Home` scrollen NICHT, solange ein Formularfeld fokussiert ist** — die Keys gehen
ins Feld statt an den Viewport, und der Blur-per-Neutralklick fokussiert oft direkt das
nächste Feld. Auch `(wm-click 4)`/`(wm-click 5)` (Mausrad-Buttons) bewegten nichts.
**Verlässlich funktioniert der Tab-Walk:** ein Feld fokussieren und dann mehrfach
`(wm-send-key 0 "Tab")` — der Browser scrollt das jeweils fokussierte Element
automatisch in den Sichtbereich, bis man beim Submit-Button ankommt. Tabben aktiviert
Radios/Toggles/Buttons nicht (nur Space/Enter täte das), ist also sicher zum
Durchlaufen. Bis `wm-scroll` gefixt ist, ist Tab-Walk der robusteste Weg durch lange
Formulare.

## Akzeptanz
`(wm-scroll 0 3)` mit Pointer über dem Content scrollt eine lange Seite sichtbar nach
unten (Vorher/Nachher-Screenshot unterscheidet sich); negatives `dy` scrollt hoch.

## Status: FIX IMPLEMENTIERT (2026-08-31)
`wm-scroll` interpretiert dx/dy jetzt als **Wheel-Notches** (1 = ein Mausrad-Klick)
und sendet im selben Frame `value120` (dx*120, diskret — darauf hört Firefox/Zen)
plus den kontinuierlichen Wert (~15 px/Notch), Quelle Wheel. Umsetzung in
`src/state.rs` (`SyntheticAction::Scroll`). Headless-Smoke: Kommando wird
angenommen; Sichtprüfung gegen echte Zen-Langseite steht noch aus (Akzeptanz oben).

## Real-World-Verifikation (2026-09-01, nested Session + Zen + Ashby)
Nested `run-nested`-Session (neues Binary), frisches Zen-Profil, echte Seiten:
Wikipedia-Langseite: `(wm-scroll 0 5)` und `(wm-scroll 0 8)` scrollen die Seite
sichtbar (vorher No-op). value120-Fix bestätigt. **VERIFIZIERT, kann zu.**

#!/bin/sh
exec guile --no-auto-compile -L scheme -s "$0" "$@"
!#
;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Generate a dependency-free local HTML index for references and demos.

(use-modules (srfi srfi-1)
             (minde commands)
             (minde command-catalog))

(define (read-one path)
  (call-with-input-file path
    (lambda (port)
      (let ((datum (read port)))
        (unless (eof-object? (read port))
          (error "trailing data" path))
        datum))))

(define (field name scenario)
  (let ((entry (assq name (cdr scenario))))
    (unless entry (error "missing scenario field" name))
    (cadr entry)))

(define data (read-one "demos/scenarios.scm"))
(define scenarios (cddr data))
(register-builtin-command-schemas!)

(define (commands-for id)
  (filter (lambda (name)
            (eq? id (command-demo-id (command-ref name))))
          (command-names)))

(define (html value)
  (call-with-output-string
    (lambda (port)
      (string-for-each
       (lambda (character)
         (case character
           ((#\&) (display "&amp;" port))
           ((#\<) (display "&lt;" port))
           ((#\>) (display "&gt;" port))
           ((#\") (display "&quot;" port))
           (else (write-char character port))))
       value))))

(display "<!doctype html>\n<html lang=\"en\" itemscope itemtype=\"https://schema.org/TechArticle\">\n<head>\n")
(display "<meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n")
(display "<title>Minde field manual</title>\n")
(display "<style>\n")
(display ":root{--bg:#fff;--ink:#0a0a0a;--ink2:#1d1d1d;--muted:#565656;--faint:#8a8a8a;--soft:#d4d4d4;--term:#000;--termfg:#ededed;--body:'IBM Plex Mono',ui-monospace,monospace;--display:'Space Mono','IBM Plex Mono',ui-monospace,monospace}*{box-sizing:border-box}html{scroll-behavior:smooth}body{margin:0;background:var(--bg);color:var(--ink);font:13px/1.43 var(--body)}a{color:inherit;text-underline-offset:2px}a:focus-visible,video:focus-visible{outline:2px solid var(--ink);outline-offset:3px}.page{width:min(1060px,calc(100% - 36px));margin:auto}.coord,.mast,.colophon{display:flex;justify-content:space-between;gap:20px;padding:7px 0;border-bottom:1px solid var(--ink);font:700 10px/1.2 var(--display);letter-spacing:.13em;text-transform:uppercase}.mast{padding:12px 0;border-bottom:2px solid var(--ink)}h1,h2,h3{font-family:var(--display);margin:0}h1{font-size:clamp(25px,4vw,38px);line-height:1.08;letter-spacing:-.03em;padding:22px 0 12px}h2{display:flex;justify-content:space-between;border-top:2px solid var(--ink);padding:9px 0 7px;font-size:15px;letter-spacing:.08em;text-transform:uppercase}h3{font-size:13px}.addr{font:10px var(--body);color:var(--muted);letter-spacing:.08em}.abstract{font-style:italic;margin:0 0 16px;max-width:78ch}.dict{display:grid;grid-template-columns:repeat(4,1fr);border-top:1px solid var(--ink);border-bottom:1px solid var(--ink);margin-bottom:18px}.dict div{padding:8px 10px;border-right:1px solid var(--soft)}.dict div:last-child{border:0}.dict b{display:block;font:700 9px var(--display);letter-spacing:.12em;text-transform:uppercase;color:var(--muted);margin-bottom:3px}.cols{columns:2;column-gap:34px;column-rule:1px solid var(--soft);text-align:justify;hyphens:auto;margin:12px 0 20px}.cols p{margin-top:0}.index{width:100%;border-collapse:collapse;border-top:2px solid var(--ink);border-bottom:2px solid var(--ink);margin:10px 0 22px}.index th{text-align:left;font-size:10px;letter-spacing:.1em;text-transform:uppercase;color:var(--muted);border-bottom:1px solid var(--ink);padding:5px 10px 5px 0}.index td{padding:5px 10px 5px 0;border-top:1px dotted var(--soft);vertical-align:top}.index caption{caption-side:bottom;text-align:left;padding-top:5px;font-size:10px;text-transform:uppercase;letter-spacing:.1em;color:var(--muted)}.demos{display:grid;grid-template-columns:1fr 1fr;gap:0 28px}.demo{break-inside:avoid;border-top:1px solid var(--ink);padding:9px 0 16px}.demo header{display:flex;justify-content:space-between;gap:12px;align-items:baseline;margin-bottom:7px}.demo-id{font-size:10px;color:var(--muted)}video{display:block;width:100%;background:#000;aspect-ratio:8/5;border:0}.trigger{margin:6px 0;font-size:11px}.trigger code,code{font-family:var(--body)}.links{display:flex;gap:14px;font-size:10px;text-transform:uppercase;letter-spacing:.08em}.term{background:var(--term);color:var(--termfg);padding:11px 13px;overflow:auto;font:11.5px/1.55 var(--body);margin:10px 0 20px}.colophon{margin-top:22px;border-top:2px solid var(--ink);border-bottom:0;padding:12px 0 24px}@media(max-width:720px){.dict{grid-template-columns:1fr 1fr}.dict div:nth-child(2){border-right:0}.cols{columns:1}.demos{grid-template-columns:1fr}.page{width:min(100% - 24px,1060px)}}@media(prefers-reduced-motion:reduce){html{scroll-behavior:auto}}@media print{video{display:none}.term{background:#fff;color:#000;border:1px solid #000}.page{width:100%}}\n")
(display "</style></head><body><main class=\"page\">\n")
(display "<div class=\"coord\"><span>A0 · A1 · A2 · A3</span><span>SHEET 08 · REV WORKING</span></div>\n")
(display "<header class=\"mast\"><span>Minde / Documentation node</span><nav><a href=\"#reference\">Reference</a> · <a href=\"#demos\">Demos</a> · <a href=\"#build\">Build</a></nav></header>\n")
(display "<h1 itemprop=\"headline\">Minde field manual</h1><p class=\"abstract\"><b>Abstract.</b> Generated, offline-first reference index for the live Scheme API, accepted default keymap, and deterministic command demonstrations. Source metadata and scenario IDs are the authority; this file is a build product.</p>\n")
(format #t "<div class=\"dict\"><div><b>Document</b>DOC-08</div><div><b>API modules</b>8</div><div><b>Command scenarios</b>~a</div><div><b>Media mode</b>opt-in / local</div></div>\n" (length scenarios))
(display "<section id=\"reference\" data-unit=\"R-01\"><h2><span>1 Reference register</span><span class=\"addr\">ADDR 0x01</span></h2><div class=\"cols\"><p>The API reference enumerates live Guile interfaces and merges command metadata. The keymap reference loads the portable configuration against inert compositor operations and walks the resulting key tables. Generated files must not be edited directly.</p><p>Entries marked <code>non-visual</code> need executable transcript coverage rather than decorative video. Missing docstrings remain visible until the public boundary is curated and source documentation is complete.</p></div>\n")
(display "<table class=\"index\"><thead><tr><th>ID</th><th>Document</th><th>Source</th><th>State</th></tr></thead><tbody>\n")
(display "<tr><td>GUIDE-START</td><td><a href=\"../tutorial.md\">First session</a></td><td>maintained prose</td><td>working</td></tr>\n")
(display "<tr><td>GUIDE-CONFIG</td><td><a href=\"../configuration.md\">Configuration</a></td><td>maintained prose</td><td>working</td></tr>\n")
(display "<tr><td>GUIDE-MODEL</td><td><a href=\"../concepts.md\">Concepts</a></td><td>maintained prose</td><td>working</td></tr>\n")
(display "<tr><td>REF-API</td><td><a href=\"api-reference.md\">Scheme API reference</a></td><td>live module exports + docstrings</td><td>generated / debt visible</td></tr>\n")
(display "<tr><td>REF-KEY</td><td><a href=\"keybindings.md\">Default keymap</a></td><td>loaded key tables</td><td>generated</td></tr>\n")
(display "<tr><td>REF-MAN</td><td><a href=\"demo-manifest.json\">Demo manifest</a></td><td>command catalog + scenarios</td><td>generated</td></tr>\n")
(display "<tr><td>GUIDE-IPC</td><td><a href=\"../ipc-eww.md\">IPC and Eww</a></td><td>schema-v1 status contract</td><td>working</td></tr>\n")
(display "<tr><td>GUIDE-DEBUG</td><td><a href=\"../debugging.md\">Debugging</a></td><td>retained evidence workflow</td><td>working</td></tr>\n")
(display "<tr><td>GUIDE-TEST</td><td><a href=\"../testing.md\">Verification</a></td><td>complete command index</td><td>working</td></tr>\n")
(display "<tr><td>GUIDE-ARCH</td><td><a href=\"../architecture.md\">Architecture</a></td><td>Rust/Guile boundary</td><td>working</td></tr>\n")
(display "<tr><td>GUIDE-SEC</td><td><a href=\"../security.md\">Security model</a></td><td>trust boundary</td><td>working</td></tr>\n")
(display "<tr><td>GUIDE-HW</td><td><a href=\"../hardware-validation.md\">Hardware validation</a></td><td>owner checklist</td><td>manual</td></tr>\n")
(display "<tr><td>GUIDE-DEMO</td><td><a href=\"../demonstrations.md\">Capture guide</a></td><td>maintained prose</td><td>working</td></tr>\n")
(display "</tbody><caption>Table 1 — generated and maintained documentation nodes.</caption></table></section>\n")
(display "<section id=\"demos\" data-unit=\"D-01\"><h2><span>2 Command demonstrations</span><span class=\"addr\">ADDR 0x02</span></h2><div class=\"demos\">\n")
(for-each
 (lambda (scenario)
   (let* ((id (field 'id scenario))
          (stem (symbol->string id))
          (title (field 'title scenario))
          (trigger (field 'trigger scenario))
          (commands (commands-for id)))
     (format #t "<article class=\"demo\" id=\"demo-~a\" data-unit=\"DEMO-~a\"><header><h3>~a</h3><span class=\"demo-id\">[~a]</span></header>"
             stem stem (html title) stem)
     (format #t "<video controls preload=\"metadata\" poster=\"../../build/demos/~a.png\" aria-label=\"~a\"><source src=\"../../build/demos/~a.webm\" type=\"video/webm\">Video unavailable; run make demos.</video>"
             stem (html title) stem)
     (format #t "<p class=\"trigger\">INPUT / <code>~a</code><br>API / <code>~a</code></p>"
             (html trigger)
             (html (string-join (map symbol->string commands) ", ")))
     (format #t "<div class=\"links\"><a href=\"../../build/demos/~a.webm\">WebM</a><a href=\"../../build/demos/~a.png\">Poster</a><a href=\"../../build/demos/~a.txt\">Transcript</a></div></article>\n"
             stem stem stem)))
 scenarios)
(display "</div></section>\n")
(display "<section id=\"build\" data-unit=\"B-01\"><h2><span>3 Reproduction</span><span class=\"addr\">ADDR 0x03</span></h2><div class=\"cols\"><p>Reference generation is fast and does not launch graphical programs. Media capture is separate, sequential, and bounded to one encoder. Both paths use the same command catalog and scenario file.</p><p>Open this file directly after generation; it has no network dependency. Video controls use browser-native behavior and every clip has a poster and text transcript fallback.</p></div><pre class=\"term\">$ make docs\n$ make check-docs\n$ make demos\n$ make check-demos</pre></section>\n")
(display "<footer class=\"colophon\"><span>END OF DOCUMENT · GENERATED FILE</span><span>MODE 1 · OFFLINE HTML</span></footer></main></body></html>\n")

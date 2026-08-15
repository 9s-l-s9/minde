;;; SPDX-License-Identifier: GPL-3.0-or-later

(use-modules (haunt artifact)
             (haunt builder assets)
             (haunt html)
             (haunt site))

(define %source-url "https://github.com/9s-l-s9/minde")

(define (feature title claim)
  `(article (@ (class "feature"))
    (h3 ,title)
    (p ,claim)))

(define (landing-page site posts)
  (list
   (serialized-artifact
    "index.html"
    `((doctype "html")
      (html (@ (lang "en"))
       (head
        (meta (@ (charset "utf-8")))
        (meta (@ (name "viewport") (content "width=device-width,initial-scale=1")))
        (meta (@ (name "theme-color") (content "#0c0b09")))
        (meta (@ (name "description") (content "Minde is a Wayland compositor controlled through live, inspectable Guile Scheme.")))
        (title "minde — a programmable Wayland compositor")
        (link (@ (rel "stylesheet") (href "assets/site.css"))))
       (body
        (header (@ (class "hero"))
         (h1 "MINDE")
         (p (@ (class "lexical"))
          (span (@ (class "pronunciation")) "/ˈmin.dɛ/")
          " (verb · Lojban)")
         (div (@ (class "rule")))
         (p (@ (class "definition"))
          "To command a desktop through live, inspectable Scheme."))
        (main
         (section (@ (class "section"))
          (p (@ (class "lead"))
           "Minde is a Wayland compositor for people who want to understand
            and alter the machinery they use all day.  Smithay owns the
            protocol, rendering, and input; Guile owns policy.  Inspect it,
            redefine it, or drive it over local IPC while the session is
            alive."))
         (section (@ (class "section"))
          ,(feature "Evaluate"
            "Keymaps, hooks, placement, groups, and frames are live Guile
             procedures — change behavior without restarting the session.")
          ,(feature "Compose"
            "Groups, heads, and explicit frames instead of a fixed layout
             algorithm; the window model descends from StumpWM.")
          ,(feature "Interrogate"
            "The local IPC interface exposes state, evaluation, status, and
             structured events to scripts and Eww.")
          ,(feature "Hack safely"
            "The nested backend exercises the real policy layer without
             touching DRM, your VT, or the display manager."))
         (section (@ (class "section"))
          (p "Try it without touching your session:")
          (pre (code "guix shell -m manifest.scm\nscripts/run-nested"))
          (p (@ (class "links"))
           (a (@ (href ,%source-url)) "Source")
           " · "
           (a (@ (href "https://github.com/9s-l-s9/minde/blob/main/doc/concepts.md"))
              "Window model")
           " · "
           (a (@ (href "https://github.com/9s-l-s9/minde#trying-it"))
              "Trying it"))))
        (footer
         (p "Minde · GPL-3.0-or-later · Wayland outside, Guile inside.")))))
    sxml->html)))

(site #:title "Minde"
      #:domain "9s-l-s9.github.io/minde"
      #:builders (list landing-page (static-directory "assets"))
      #:build-directory "_site")

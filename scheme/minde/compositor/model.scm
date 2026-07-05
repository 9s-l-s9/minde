;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Internal mutable records shared by compositor policy modules.

(define-module (minde compositor model)
  #:use-module (srfi srfi-9)
  #:export (make-frame frame-node? frame-x set-frame-x! frame-y set-frame-y!
            frame-w set-frame-w! frame-h set-frame-h!
            frame-window-ids set-frame-window-ids!
            frame-current-window set-frame-current-window!
            make-split split-node? split-orientation split-ratio set-split-ratio!
            split-child-a set-split-child-a! split-child-b set-split-child-b!
            make-group-record group? group-name set-group-name!
            group-tree set-group-tree! group-current-frame set-group-current-frame!
            group-last-window set-group-last-window!
            group-shown-window set-group-shown-window!
            group-last-frame set-group-last-frame!
            group-shown-frame set-group-shown-frame!
            group-heads group-loaded-head set-group-loaded-head!
            group-floats set-group-floats! group-float? set-group-float?!))

(define-record-type <frame>
  (make-frame x y w h window-ids current-window)
  frame-node?
  (x frame-x set-frame-x!) (y frame-y set-frame-y!)
  (w frame-w set-frame-w!) (h frame-h set-frame-h!)
  (window-ids frame-window-ids set-frame-window-ids!)
  (current-window frame-current-window set-frame-current-window!))

(define-record-type <split>
  (make-split orientation ratio child-a child-b)
  split-node?
  (orientation split-orientation)
  (ratio split-ratio set-split-ratio!)
  (child-a split-child-a set-split-child-a!)
  (child-b split-child-b set-split-child-b!))

(define-record-type <group>
  (make-group-record name tree current-frame
                     last-window shown-window last-frame shown-frame
                     heads loaded-head floats float?)
  group?
  (name group-name set-group-name!)
  (tree group-tree set-group-tree!)
  (current-frame group-current-frame set-group-current-frame!)
  (last-window group-last-window set-group-last-window!)
  (shown-window group-shown-window set-group-shown-window!)
  (last-frame group-last-frame set-group-last-frame!)
  (shown-frame group-shown-frame set-group-shown-frame!)
  (heads group-heads)
  (loaded-head group-loaded-head set-group-loaded-head!)
  (floats group-floats set-group-floats!)
  (float? group-float? set-group-float?!))

;; Public SRFI-9 accessors re-exported by the policy modules are syntax
;; bindings, so their API descriptions live beside their record definitions.
(define %api-binding-documentation
  '((frame-window-ids . "Returns the window identifiers assigned to frame RECORD.")
    (frame-current-window . "Returns frame RECORD's selected window identifier, or #f.")
    (group-name . "Returns group RECORD's padded display name.")
    (group-floats . "Returns the floating window identifiers owned by group RECORD.")
    (group-float? . "Returns true when group RECORD is a floating group.")))

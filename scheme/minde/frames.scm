;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Stable, curated manual-frame API. Event adapters, persistence internals,
;;; mutable model access, and Rust synchronization stay in the private
;;; (minde compositor frames) implementation module.

(define-module (minde frames)
  #:use-module (minde compositor frames)
  #:export (frame-api-groups frame-api-tags)
  #:re-export (;; Frame topology and layout.
               split-frame-horizontal!
               split-frame-vertical!
               remove-split!
               collapse-to-one-frame!
               clear-current-frame!
               hsplit-equally!
               vsplit-equally!
               resize-frame!
               balance-frames!
               apply-layout-spec!
               set-gaps!

               ;; Focus and navigation.
               focus-next-frame!
               focus-previous-frame!
               focus-next-window!
               focus-previous-window!
               focus-next-window-in-frame!
               focus-previous-window-in-frame!
               other-window!
               other-frame!
               move-focus!
               focus-sibling-frame!
               focus-frame-by-index!

               ;; Window placement and numbering.
               pull-hidden-next!
               pull-hidden-previous!
               pull-window-from-other-frame!
               move-window!
               exchange-windows!
               select-window-by-number!
               pull-window-by-number!
               pull-window-by-id!
               renumber-window!
               repack-window-numbers!

               ;; Window state and lifecycle.
               fullscreen!
               kill-current-window!
               toggle-always-on-top!
               toggle-always-show!
               rename-window!
               unmaximize!

               ;; Floating windows.
               flatten-floats!
               set-window-gravity!
               set-float-geometry!
               float-this!
               float-window!
               unfloat-window!

               ;; Keyboard and pointer input.
               ratwarp!
               move-pointer-to-corner!
               window-send-string
               send-key
               meta
               send-escape
               define-remapped-keys!
               unbind-remapped-keys!
               toggle-remapped-keys!
               ratrelwarp
               ratclick!
               idle-ms

               ;; Visual interaction and diagnostics.
               echo-windows-string
               show-window-properties!
               show-frame-overlays!
               clear-frame-overlays!
               expose-enter!
               expose-pick!

               ;; Persistence and inspection.
               dump-frames
               restore-frames!
               dump-layout-spec))

;; This public metadata is both a navigational aid and an executable contract:
;; tests require every operation exported above to occur in exactly one group.
(define frame-api-groups
  '((topology-and-layout
     split-frame-horizontal! split-frame-vertical! remove-split!
     collapse-to-one-frame! clear-current-frame! hsplit-equally!
     vsplit-equally! resize-frame! balance-frames! apply-layout-spec! set-gaps!)
    (focus-and-navigation
     focus-next-frame! focus-previous-frame! focus-next-window!
     focus-previous-window! focus-next-window-in-frame!
     focus-previous-window-in-frame! other-window! other-frame! move-focus!
     focus-sibling-frame! focus-frame-by-index!)
    (window-placement
     pull-hidden-next! pull-hidden-previous! pull-window-from-other-frame!
     move-window! exchange-windows! select-window-by-number!
     pull-window-by-number! pull-window-by-id! renumber-window!
     repack-window-numbers!)
    (window-lifecycle
     fullscreen! kill-current-window! toggle-always-on-top! toggle-always-show!
     rename-window! unmaximize!)
    (floating-windows
     flatten-floats! set-window-gravity! set-float-geometry! float-this!
     float-window! unfloat-window!)
    (input-and-pointer
     ratwarp! move-pointer-to-corner! window-send-string send-key meta send-escape
     define-remapped-keys! unbind-remapped-keys! toggle-remapped-keys!
     ratrelwarp ratclick! idle-ms)
    (visual-interaction
     echo-windows-string show-window-properties! show-frame-overlays!
     clear-frame-overlays! expose-enter! expose-pick!)
    (persistence-and-inspection
     dump-frames restore-frames! dump-layout-spec)))

;; Tags deliberately overlap. They let documentation and discovery tools show
;; cross-cutting concerns without forcing an operation into several groups.
(define frame-api-tags
  '((layout
     split-frame-horizontal! split-frame-vertical! remove-split!
     collapse-to-one-frame! hsplit-equally! vsplit-equally! resize-frame!
     balance-frames! apply-layout-spec! set-gaps! dump-layout-spec)
    (focus
     focus-next-frame! focus-previous-frame! focus-next-window!
     focus-previous-window! focus-next-window-in-frame!
     focus-previous-window-in-frame! other-window! other-frame! move-focus!
     focus-sibling-frame! focus-frame-by-index!)
    (window-placement
     pull-hidden-next! pull-hidden-previous! pull-window-from-other-frame!
     move-window! exchange-windows! select-window-by-number!
     pull-window-by-number! pull-window-by-id! renumber-window!
     repack-window-numbers!)
    (window-state
     clear-current-frame! fullscreen! kill-current-window!
     toggle-always-on-top! toggle-always-show! rename-window! unmaximize!)
    (floating
     flatten-floats! set-window-gravity! set-float-geometry! float-this!
     float-window! unfloat-window!)
    (keyboard
     window-send-string send-key meta send-escape define-remapped-keys!
     unbind-remapped-keys! toggle-remapped-keys!)
    (pointer ratwarp! move-pointer-to-corner! ratrelwarp ratclick!)
    (visual
     echo-windows-string show-window-properties! show-frame-overlays!
     clear-frame-overlays! expose-enter! expose-pick!)
    (persistence dump-frames restore-frames! dump-layout-spec)
    (inspection echo-windows-string idle-ms show-window-properties! dump-frames
                dump-layout-spec)))

(define %api-binding-documentation
  '((frame-api-groups .
     "Returns the categorized public frame operations as (CATEGORY NAME ...) entries.")
    (frame-api-tags .
     "Returns overlapping frame API tags as (TAG NAME ...) entries.")))

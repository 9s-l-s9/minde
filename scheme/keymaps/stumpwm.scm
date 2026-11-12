;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Optional historical keymap. Loaded only when explicitly selected.

(set! %prefix-bindings (make-hash-table))
(set! %keybindings (make-hash-table))
(set! %global-binding-docs (make-hash-table))
(set! %binding-docs (make-weak-key-hash-table))
(set! %binding-submaps (make-weak-key-hash-table))
(bind-prefix-key! "Return" run-prompt! "run command")
;; r: StumpWM exec -- native prompt with PATH completion (TAB). An
;; external launcher (fuzzel/bemenu) is no longer needed but still works
;; as a layer-shell client if you prefer one.
(bind-prefix-key! "r" (lambda () (run-prompt!)))

(bind-prefix-key! "v" (lambda () (guard-manual-tiling split-frame-vertical!)))
(bind-prefix-key! "h" (lambda () (guard-manual-tiling split-frame-horizontal!)))
(bind-prefix-key! "c" (lambda () (guard-manual-tiling remove-split!)))
(bind-prefix-key! "n" (lambda () (focus-next-frame!)))
;; f: StumpWM `next` -- cycle through ALL of the group's windows,
;; emacs-buffer-style, jumping frames as needed.
(bind-prefix-key! "f" (lambda () (focus-next-window!)))
;; p: StumpWM `pull` -- dig the next hidden window out into this frame.
(bind-prefix-key! "p" (lambda () (pull-hidden-next!)))
;; o: cycle only within the current frame's own window stack. Print Print
;; (prefix key twice) does the same -- nobody sends a literal Print to a
;; client anyway; rebind or unbind "Print" here to restore the StumpWM
;; literal-forward behavior.
(bind-prefix-key! "o" (lambda () (focus-next-window-in-frame!)))
(bind-prefix-key! "Print" (lambda () (focus-next-window-in-frame!)))
(bind-prefix-key! "k" (lambda () (close-current-window!)))
(bind-prefix-key! "d" (lambda () (close-current-window!))) ; StumpWM delete-window
(bind-prefix-key! "g" (lambda () (switch-to-next-group!)))
;; G: new group -- prompts for a name; empty input auto-names (roman).
(bind-prefix-key! "G"
  (lambda ()
    (read-one-line "new group: "
      (lambda (name)
        (let ((g (if (string-null? name)
                     (create-auto-named-group!)
                     (create-group! (string-append " " name " ")))))
          (echo (string-append "new group:" (group-name g)))))
      #:history 'group)))
;; y: StumpWM `info` -- echo the current window and group.
(bind-prefix-key! "y" (lambda ()
                        (let ((id (current-frame-window)))
                          (echo (if id
                                    (format #f "~a  [~a]" (window-title id)
                                            (string-trim-both (current-group-name)))
                                    "no window")))))
(bind-prefix-key! "m" (lambda () (move-window-to-next-group!)))

;; ---------------------------------------------------------------------
;; Window numbers + navigation (StumpWM parity, conflict-free keys)
;; ---------------------------------------------------------------------

;; 0-9: jump to window by number; C-0..C-9: pull it here instead.
(for-each
 (lambda (n)
   (let ((k (number->string n)))
     (bind-prefix-key! k (lambda () (select-window-by-number! n)))
     (bind-prefix-key! (string-append "C-" k)
                       (lambda () (pull-window-by-number! n)))))
 (iota 10))

;; Arrows: directional frame focus; M-arrows move the window along;
;; S-arrows swap windows with the neighbor frame.
(for-each
 (lambda (pair)
   (let ((name (car pair)) (dir (cdr pair)))
     (bind-prefix-key! name (lambda () (move-focus! dir)))
     (bind-prefix-key! (string-append "M-" name) (lambda () (move-window! dir)))
     (bind-prefix-key! (string-append "S-" name) (lambda () (exchange-windows! dir)))))
 '(("Left" . left) ("Right" . right) ("Up" . up) ("Down" . down)))

;; Tab: toggle to the previously focused window (emacs C-x b RET);
;; S-Tab (keysym ISO_Left_Tab): toggle to the previous frame; u: toggle
;; to the previous group (StumpWM gother).
(bind-prefix-key! "Tab" (lambda () (other-window!)))
(bind-prefix-key! "ISO_Left_Tab" (lambda () (other-frame!)))
(bind-prefix-key! "u" (lambda () (switch-to-last-group!)))

;; W: StumpWM `windows` -- echo the numbered window list.
(bind-prefix-key! "W" (lambda () (echo (echo-windows-string))))

;; C: StumpWM `only` -- collapse all splits, keep every window;
;; Delete: fclear -- show this frame empty (its windows stay, hidden).
(bind-prefix-key! "C" (lambda () (collapse-to-one-frame!)))
(bind-prefix-key! "Delete" (lambda () (clear-current-frame!)))

;; Reverse cycling, mirroring f/p/n/o forwards.
(bind-prefix-key! "C-f" (lambda () (focus-previous-window!)))
(bind-prefix-key! "C-p" (lambda () (pull-hidden-previous!)))
(bind-prefix-key! "C-n" (lambda () (focus-previous-frame!)))
(bind-prefix-key! "C-o" (lambda () (focus-previous-window-in-frame!)))

;; ---------------------------------------------------------------------
;; Help, eval prompt, message history, marks, group management
;; (StumpWM parity)
;; ---------------------------------------------------------------------

;; While the prefix (or a submap) is armed, ? echoes its bindings and
;; stays armed -- docs for the existing map so that's actually useful:
(for-each
 (lambda (p) (set-binding-doc! %prefix-bindings (car p) (cdr p)))
 '(("Return" . "run program (prompt)") ("r" . "run program (prompt)")
   ("v" . "vsplit") ("h" . "hsplit") ("c" . "remove split")
   ("n" . "next frame") ("f" . "next window (group)")
   ("p" . "pull hidden window") ("o" . "next window in frame")
   ("k" . "close window") ("d" . "close window")
   ("g" . "next group") ("G" . "new group (prompt)")
   ("m" . "move window to next group") ("y" . "window info")
   ("l" . "windowlist (prompt)") ("P" . "misc submap")
   ("R" . "reload configuration") ("L" . "lock screen") ("Q" . "quit (asks)")
   ("s" . "resize mode") ("F" . "float/unfloat window")
   ("M-F" . "apply layout (prompt)")
   ("Tab" . "last window") ("ISO_Left_Tab" . "last frame")
   ("u" . "last group") ("W" . "window list echo")
   ("C" . "only: collapse splits") ("Delete" . "fclear: empty frame")
   ("z" . "command mode (RET/C-g exits)")))

;; F1: describe-key -- press it, then any key, get told what it does.
(bind-prefix-key! "F1" (lambda () (describe-key!)) "describe next key")

;; z: command mode -- every key acts as a prefix key until RET/C-g.
(bind-prefix-key! "z" (lambda () (command-mode!)) "command mode (RET/C-g exits)")
(bind-prefix-key! "colon" (lambda () (eval-prompt!)) "eval scheme (prompt)")

;; C-m: re-show the last message (StumpWM lastmsg).
(bind-prefix-key! "C-m"
  (lambda () (echo (or (last-message) "no messages")))
  "last message again")

;; Q now asks before quitting (StumpWM quit-confirm); logout! (minde
;; session) owns the actual prompt-then-wm-quit shape, so this, the
;; portable "s q" binding, and the colon prompt all agree on one
;; implementation.
(bind-prefix-key! "Q" (lambda () (logout!)) "quit (asks)")

;; ---------------------------------------------------------------------
;; Session management (minde session): L locks; M-L suspends (locks
;; first and waits for confirmation -- see the module for why). Logout
;; is deliberately not on its own single keystroke anywhere: Q above
;; and the portable "s q" binding both go through logout!'s confirmation
;; prompt.
;; ---------------------------------------------------------------------
(bind-prefix-key! "L" (lambda () (lock-screen!)) "lock screen")
(bind-prefix-key! "M-L" (lambda () (suspend!)) "suspend (locks first)")
(bind-prefix-key! "R" reload-configuration! "reload configuration")

;; Marks: tag windows, then pull them all here at once. (Not on "t":
;; that must stay free as the literal-forward escape for the default
;; C-t prefix.)
(bind-prefix-key! "x" (lambda () (mark-window-toggle!)) "mark/unmark window")
(bind-prefix-key! "M-x" (lambda () (pull-marked!)) "pull marked windows here")
(bind-prefix-key! "C-x" (lambda () (clear-marks!)) "clear marks")
(bind-prefix-key! "M-g" (lambda () (gselect!)) "switch group (prompt)")
(bind-prefix-key! "M-m" (lambda () (move-current-window-to-next-group-and-follow!)) "move window + follow")

;; Fullscreen / force kill / banish (see (minde frames)).
(bind-prefix-key! "M-f" (lambda () (fullscreen!)) "fullscreen toggle")
(bind-prefix-key! "K" (lambda () (kill-current-window!)) "kill window (force)")
(bind-prefix-key! "B" (lambda () (move-pointer-to-corner!)) "banish pointer")
(bind-prefix-key! "C-c" (lambda () (flash-current-frame!)) "flash current frame")
(bind-prefix-key! "C-u" (lambda () (next-urgent!)) "jump to urgent window")
(bind-prefix-key! "M-c" (lambda () (copy-last-message!)) "copy last message")

;; ---------------------------------------------------------------------
;; Multi-head (multi-monitor): each group has a frame tree per head;
;; S cycles heads, M-s toggles to the last one. Directional focus
;; (prefix arrows) also crosses monitor edges. Users who prefer one big
;; tree spanning all monitors can (set-head-mode! 'span).
;;
;; The portable default binds heads under "o" instead.
;; ---------------------------------------------------------------------

(bind-prefix-key! "S" (lambda () (focus-next-head!)) "next head (monitor)")
(bind-prefix-key! "M-s" (lambda () (focus-last-head!)) "last head (monitor)")

(bind-prefix-key! "s"
  (lambda ()
    (call-if-bound 'wm-message %resize-help 0)
    %resize-map))

;; F floats/unfloats (the user's old StumpWM float-this binding); the
;; layout prompt moved to M-F.
(bind-prefix-key! "F"
  (lambda ()
    (float-this!)
    ;; Floating in/out of a dynamic group changes the tiled set.
    (when (dynamic-group?) (retile-dynamic!)))
  "float/unfloat window")
(bind-prefix-key! "M-F" (lambda () (layout-prompt!)) "apply layout (prompt)")

;; ---------------------------------------------------------------------
;; Placement rules: route windows to a group/frame by app-id or title
;; substring when they map (StumpWM define-frame-preference). Examples:
;; ---------------------------------------------------------------------

;; (add-placement-rule! "zen" #:group "II" #:frame 0)
;; (add-placement-rule! "emacs" #:frame 1 #:follow? #t)
;; (add-placement-rule! "Picture-in-Picture" #:float? #t)

;; Miscellaneous built-in operations. Personal launchers and desktop policy
;; belong in a user configuration layered over this optional map.
(bind-prefix-key! "P"
  (make-keymap
   ;; s: save the current frame tree as a named layout (Print M-F applies).
   "s" save-layout-prompt!
   ;; r: rename the current group; k: delete it (windows move on).
   "r" (lambda ()
         (read-one-line "rename group: "
           (lambda (name) (rename-current-group! name))
           #:history 'group))
   "k" (lambda () (delete-current-group!))
   ;; n: rename the current window (StumpWM title).
   "n" (lambda ()
         (read-one-line "window name: "
           (lambda (name) (rename-window! name))))
   ;; p: re-apply placement rules to existing windows.
   "p" (lambda () (place-existing-windows!))
   ;; f: unfloat every float of this group; t: always-on-top toggle;
   ;; y: always-show (sticky, follows every group switch).
   "f" (lambda () (flatten-floats!))
   "t" (lambda () (toggle-always-on-top!))
   "y" (lambda () (toggle-always-show!))
   ;; i: window properties echo; d: date; V: version; M: modifiers.
   "i" (lambda () (show-window-properties!))
   "d" (lambda () (echo-date!))
   "V" (lambda () (version!))
   "M" (lambda () (modifiers!))
   ;; u: unmaximize toggle; g: gravity of the unmaximized window.
   "u" (lambda () (unmaximize!))
   "g" (lambda ()
         (read-one-line "gravity: "
           (lambda (s)
             (unless (string-null? s)
               (set-window-gravity! (string->symbol s))))
           #:completions %gravity-names))
   ;; R/F: remember/forget a persistent placement rule for this window.
   "R" (lambda () (remember!))
   "F" (lambda () (forget!))
   ;; D/O: dump / restore the whole desktop (groups + frame layouts).
   "D" dump-desktop-prompt!
   "O" restore-desktop-prompt!))

(bind-prefix-key! "l" (lambda () (windowlist!)))

;; Bindings: windowlist variants at the top level, group management in
;; the M-G submap, window odds and ends in the P submap.
(bind-prefix-key! "C-l" (lambda () (frame-windowlist!)) "frame windowlist (menu)")
(bind-prefix-key! "M-l" (lambda () (windowlist-by-class!)) "windowlist by class (menu)")
(bind-prefix-key! "M-p" (lambda () (pull-from-windowlist!)) "pull from windowlist (menu)")
(bind-prefix-key! "M-w" (lambda () (select-window-by-name!)) "select window by name (prompt)")
(bind-prefix-key! "M-t" (lambda () (select-floating-window!)) "select floating window (menu)")
(bind-prefix-key! "C-r" (lambda () (redisplay!)) "redisplay windows")

(bind-prefix-key! "M-G"
  (make-keymap
   "n" (lambda () (gnewbg-prompt!))
   "N" (lambda () (gnewbg-float-prompt!))
   "m" (lambda () (gmerge-prompt!))
   "M" (lambda () (gmove-marked-prompt!))
   "o" (lambda () (delete-other-groups!))
   "g" (lambda () (echo (groups-echo-string)))
   "f" (lambda () (shift-current-window-to-next-group!))
   "b" (lambda () (shift-current-window-to-previous-group!))
   "k" (lambda () (kill-windows-current-group!))
   "K" (lambda () (kill-windows-other!))
   "d" (lambda () (gnew-dynamic-prompt!))
   "D" (lambda () (gnewbg-dynamic-prompt!))
   "r" (lambda () (rotate-windows! 'forward))
   "C-r" (lambda () (rotate-windows! 'backward))
   "s" (lambda () (rotate-stack! 'forward))
   "x" (lambda () (exchange-with-master!))
   "l" (lambda () (change-layout-prompt!))
   "S" (lambda () (change-split-ratio-prompt!))
   "t" (lambda () (retile-dynamic-group!)))
  "groups submap")
(for-each
 (lambda (p) (set-binding-doc! (hash-ref %prefix-bindings "M-G") (car p) (cdr p)))
 '(("n" . "new group in background (prompt)")
   ("N" . "new float group in background (prompt)")
   ("m" . "merge a group here (menu)")
   ("M" . "move marked windows to a group (menu)")
   ("o" . "kill all other groups")
   ("g" . "group list echo")
   ("f" . "next group, taking the window")
   ("b" . "previous group, taking the window")
   ("k" . "close all windows in this group")
   ("K" . "close all windows in other groups")
   ("d" . "new dynamic group (prompt)")
   ("D" . "new dynamic group in background (prompt)")
   ("r" . "rotate windows through master")
   ("C-r" . "rotate windows backward")
   ("s" . "rotate the stack")
   ("x" . "exchange window with master")
   ("l" . "master layout (prompt)")
   ("S" . "master split ratio (prompt)")
   ("t" . "retile")))

(bind-prefix-key! "j" (lambda () (fselect!)) "fselect: jump to frame by number")
(bind-prefix-key! "M-e" (lambda () (expose-mode!)) "expose: pick window from grid")
(bind-prefix-key! "M-o" (lambda () (focus-sibling-frame!)) "sibling frame")

(bind-prefix-key! "M-h"
  (lambda () (guard-manual-tiling
              (lambda () (split-uniformly-prompt! 'horizontal))))
  "hsplit uniformly (prompt)")
(bind-prefix-key! "M-v"
  (lambda () (guard-manual-tiling
              (lambda () (split-uniformly-prompt! 'vertical))))
  "vsplit uniformly (prompt)")
(bind-prefix-key! "C-q" (lambda () (send-raw-key!)) "send next key to window")

(bind-prefix-key! "F2" (lambda () (describe-command!)) "describe command (prompt)")
(bind-prefix-key! "F3" (lambda () (commands!)) "list all commands")
(bind-prefix-key! "F4" (lambda () (describe-function!)) "describe function (prompt)")
(bind-prefix-key! "F5" (lambda () (where-is!)) "where-is (doc search)")
(bind-prefix-key! "F6" (lambda () (describe-variable!)) "describe variable (prompt)")

;; Colon/REPL-callable, unbound by default (like StumpWM): which-key-mode!,
;; define-remapped-keys!, toggle-remapped-keys!, unbind-remapped-keys!,
;; send-key / meta / send-escape, ratrelwarp, load-module!,
;; copy-unhandled-error!, reload-configuration!.

;; Historical prefix; declarative configuration can override it.
(set-prefix-key! '() "Print")

(set! %configuration-default-file "keymaps/stumpwm.conf")
(register-configuration-layer!)

;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Built-in command metadata, kept independent from runtime procedures.

(define-module (minde command-catalog)
  #:use-module (minde commands)
  #:export (builtin-command-metadata
            register-builtin-command!
            register-builtin-command-schemas!))

;; name, arguments, category, summary, full documentation, demo id
(define %demonstrated-commands
  '((switch-to-next-group! () group
     "Focus the next group." "Focus the next group, wrapping at the end." group-next)
    (switch-to-previous-group! () group
     "Focus the previous group." "Focus the previous group, wrapping at the start." group-previous)
    (switch-to-last-group! () group
     "Return to the last focused group." "Toggle back to the previously focused group." group-last)
    (focus-next-frame! () frame
     "Focus the next frame." "Focus the next frame on the current output." frame-next)
    (focus-previous-frame! () frame
     "Focus the previous frame." "Focus the previous frame on the current output." frame-previous)
    (focus-next-window! () window
     "Focus the next group window." "Focus the next window across every frame in the group." window-next)
    (focus-previous-window! () window
     "Focus the previous group window." "Focus the previous window across every frame in the group." window-previous)
    (pull-hidden-next! () window
     "Pull the next hidden window." "Move the next hidden group window into the current frame." window-pull)
    (split-frame-horizontal! () frame
     "Split the frame into columns." "Split the current frame into left and right children." frame-split-horizontal)
    (split-frame-vertical! () frame
     "Split the frame into rows." "Split the current frame into top and bottom children." frame-split-vertical)
    (remove-split! () frame
     "Remove the current split." "Remove the current frame and merge its sibling subtree." frame-remove-split)
    (clear-current-frame! () frame
     "Hide every window in the current frame." "Leave the current frame empty without forgetting its windows." frame-clear)
    (collapse-to-one-frame! () frame
     "Collapse the current head to one frame." "Collapse all splits while retaining every window." frame-collapse)
    (focus-next-head! () output
     "Focus the next output head." "Focus the next output head, wrapping at the end." head-next)
    (focus-last-head! () output
     "Return to the last focused output head." "Toggle back to the previously focused output head." head-last)
    (gaps-on! () frame
     "Enable gaps." "Enable configured gaps without changing sizes." gaps-on)
    (gaps-off! () frame
     "Disable gaps." "Disable gaps while retaining configured sizes." gaps-off)
    (toggle-gaps! () frame
     "Toggle gaps." "Toggle configured gaps on or off." gaps-toggle)
    (reload-configuration! () configuration
     "Validate and atomically reload configuration."
     "Validate a declarative configuration in isolation and publish it only on success."
     configuration-reload)))

;; Utility and modal commands have no dedicated recorded demonstration.
;; Keep their schemas available to offline configuration validation as well
;; as to the live command palette; implementations are installed by init.scm.
(define %keymap-commands
  '((run-prompt! prompt "Run a command.")
    (command-palette! prompt "Choose a registered command.")
    (eval-prompt! prompt "Evaluate Scheme.")
    (windowlist! window "Choose a window.")
    (pull-hidden-previous! window "Pull the previous hidden window.")
    (close-current-window! window "Close the focused window.")
    (float-this! window "Float or tile the focused window.")
    (focus-previous-head! output "Focus the previous output head.")
    (gselect! group "Choose a group.")
    (move-current-window-to-next-group-and-follow! group "Move the window to the next group and follow.")
    (layout-prompt! frame "Choose a saved layout.")
    (enter-resize-mode! frame "Resize the current frame interactively.")
    (command-mode! input "Keep the prefix map armed until cancelled.")
    (lock-screen! session "Lock the session using configured policy.")
    (suspend! session "Suspend after the session locks.")
    (logout! session "Log out after confirmation.")
    (show-windows! window "Show the group window list.")
    (show-frame-labels! frame "Show frame labels briefly.")
    (fselect! frame "Choose a labeled frame.")
    (expose-mode! window "Choose a window from a temporary grid.")
    (mark-window-toggle! window "Toggle the current window's mark.")
    (pull-marked! window "Pull marked windows into the current frame.")
    (clear-marks! window "Clear all window marks.")
    (remember! placement "Remember placement of the current window.")
    (forget! placement "Forget placement of the current window.")
    (dump-desktop-prompt! persistence "Save the desktop to a chosen file.")
    (restore-desktop-prompt! persistence "Restore the desktop from a chosen file.")
    (save-layout-prompt! persistence "Save the current layout under a name.")
    (describe-key! help "Describe the next key.")
    (describe-command! help "Describe a prefix command.")
    (commands! help "List prefix commands.")
    (describe-function! help "Describe a Scheme function.")
    (describe-variable! help "Describe a Scheme variable.")
    (where-is! help "Find a binding by its description.")
    (copy-unhandled-error! help "Copy the last keybinding error.")
    (gnew-dynamic-prompt! group "Create and focus a dynamic group.")
    (gnewbg-dynamic-prompt! group "Create a dynamic group in the background.")
    (gnewbg-prompt! group "Create a group in the background.")
    (gnewbg-float-prompt! group "Create a floating group in the background.")
    (gmerge-prompt! group "Merge a group here.")
    (gmove-marked-prompt! group "Move marked windows to another group.")
    (frame-windowlist! window "Choose a window in the current frame.")
    (windowlist-by-class! window "Choose a window by application.")
    (pull-from-windowlist! window "Pull a window chosen from a list.")
    (select-floating-window! window "Choose a floating window.")
    (select-window-by-name! window "Choose a window by name.")))

(define %parameterized-keymap-commands
  '((move-focus! frame "Focus a frame in a direction."
      ((name . direction) (type . (enum left down up right))))
    (move-window! window "Move the window to a neighboring frame."
      ((name . direction) (type . (enum left down up right))))
    (exchange-windows! window "Exchange windows with a neighboring frame."
      ((name . direction) (type . (enum left down up right))))
    (select-window-by-number! window "Select a numbered window."
      ((name . number) (type . integer) (minimum . 0)))
    (pull-window-by-number! window "Pull a numbered window."
      ((name . number) (type . integer) (minimum . 0)))
    (focus-frame-by-index! frame "Select a numbered frame."
      ((name . index) (type . integer) (minimum . 0)))
    (set-head-mode! output "Select per-head or spanning frame trees."
      ((name . mode) (type . (enum per-head span))))))

(define builtin-command-metadata
  (append %demonstrated-commands
          (map (lambda (entry)
                 (list (car entry) '() (cadr entry) (caddr entry) (caddr entry) 'non-visual))
               %keymap-commands)
          (map (lambda (entry)
                 (let ((parameter (cadddr entry)))
                   (list (car entry) (list (assq-ref parameter 'name))
                         (cadr entry) (caddr entry) (caddr entry) 'non-visual
                         (list parameter))))
               %parameterized-keymap-commands)))

(define (metadata name)
  (or (assq name builtin-command-metadata)
      (error "command absent from built-in catalog" name)))

(define (register-builtin-command! name procedure)
  (let ((entry (metadata name)))
    (register-command! name procedure
                       #:arguments (list-ref entry 1)
                       #:category (list-ref entry 2)
                       #:summary (list-ref entry 3)
                       #:documentation (list-ref entry 4)
                       #:demo-id (list-ref entry 5)
                       #:parameters (and (> (length entry) 6) (list-ref entry 6)))))

(define (register-builtin-command-schemas!)
  (clear-command-registry!)
  (for-each (lambda (entry)
              (register-builtin-command! (car entry) (lambda arguments #f)))
            builtin-command-metadata))

;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Built-in command metadata, kept independent from runtime procedures.

(define-module (minde command-catalog)
  #:use-module (minde commands)
  #:export (builtin-command-metadata
            register-builtin-command!
            register-builtin-command-schemas!))

;; name, arguments, category, summary, full documentation, demo id
(define builtin-command-metadata
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
    (reload-configuration! () configuration
     "Validate and atomically reload configuration."
     "Validate a declarative configuration in isolation and publish it only on success."
     configuration-reload)))

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
                       #:demo-id (list-ref entry 5))))

(define (register-builtin-command-schemas!)
  (clear-command-registry!)
  (for-each (lambda (entry)
              (register-builtin-command! (car entry) (lambda arguments #f)))
            builtin-command-metadata))

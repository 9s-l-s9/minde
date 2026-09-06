# 06 -- The agent shows what it will do before doing it

**Request.**  "Arrange these six windows for a code review."

The agent has a proposal, but the user has opinions.  Rather than
applying a layout and asking "is this right?", the agent draws the
frames it would create over the current screen, labels which window
would go where, and asks.

Depends on PLAN.md H1 and H5; the confirmation prompt exists today.

## Session

```scheme
(define proposal '(hsplit 0.6 leaf (vsplit 0.5 leaf leaf)))

(show-proposed-layout! proposal)                 ; plan:H5: draws frame rects, no change
(highlight-window! emacs-id #:label "1")         ; plan:H5
(highlight-window! term-id  #:label "2")
(highlight-window! ff-id    #:label "3")

(read-one-line "Apply this layout? [y/n/edit] "                    ; exists (callback style)
  (lambda (answer)
    (clear-highlights!)
    (cond ((equal? answer "y")
           (apply-layout-spec! proposal)                              ; exists
           (pull-window-by-id! emacs-id) ...)
          ((equal? answer "edit")
           (annotate! #:tool 'line))                                  ; plan:H2: user draws the split they want
          (else (echo "Left as is."))))
  #:on-abort clear-highlights!)
```

If the user chooses `edit`, the agent receives the drawn line as a
shape with coordinates, converts it to a split ratio, and shows the
revised proposal.  Two rounds of this beat ten rounds of "a bit more to
the left".

## Why this matters

Most agent mistakes in a desktop are not wrong actions but right
actions on the wrong target.  Showing the target first, on the live
screen, in the compositor's own overlay, removes that class of error
without a screenshot round trip.  A dispatcher API has no way to draw
anything, so an agent using one can only act and apologise.

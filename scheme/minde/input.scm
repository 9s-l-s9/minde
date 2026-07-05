;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Stable input, key-notation, and prompt API.

(define-module (minde input)
  #:use-module (minde foundation keys)
  #:use-module (minde ui prompt)
  #:re-export (modifier->bit
               modifiers->bitmask
               key-notation
               make-key-registry
               register-key!
               lookup-key
               registered-keys
               configure-prompt-ui!
               read-one-line
               input-active?
               input-handle-key!
               input-paste!
               input-abort!))

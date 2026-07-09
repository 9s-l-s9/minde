;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Public package entry point: build only from an explicit vendored archive.

(define archive (getenv "MINDE_SOURCE_ARCHIVE"))
(define version (getenv "MINDE_VERSION"))
(define revision (getenv "MINDE_BUILD_REVISION"))

(unless (and archive (file-exists? archive))
  (error "MINDE_SOURCE_ARCHIVE must name a vendored release archive"))
(unless (and version (not (string-null? version)))
  (error "MINDE_VERSION is required"))
(unless (and revision (not (string-null? revision)))
  (error "MINDE_BUILD_REVISION is required"))

(primitive-load
 (canonicalize-path
  (string-append (dirname (current-filename)) "/../guix.scm")))

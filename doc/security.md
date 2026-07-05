# Security model

Minde is unreleased and not yet suitable as a security boundary without
owner review. The public reporting policy is in [`../SECURITY.md`](../SECURITY.md).

## Trusted components

The compositor, loaded Scheme entry point, declarative configuration, Guix
generation, and commands launched through `wm-spawn` execute with the session
user's authority. Personal configuration is code and must be reviewed like any
other executable program.

## Control socket

The main IPC socket is created with mode `0600` inside the user's runtime
directory. Requests are Scheme evaluation, so access to that socket is full
control of the compositor session. Do not proxy or expose it across users or
machines.

## Input, clipboard, and diagnostics

The compositor necessarily observes keyboard, pointer, window metadata and
clipboard protocol traffic. Logs and reports avoid clipboard contents and
redact focused title/application identifiers when requested, but owner
inspection remains mandatory before sharing diagnostic material.

## Session locking

Modern swaylock requires `ext-session-lock-v1`. Minde does not implement it.
Layer shell is not a safe substitute: a fake lock surface without exclusive
input and correct output lifecycle would create a false security boundary.

## Unsafe development interface

The supported IPC evaluator is serialized on the event-loop thread.
`MINDE_UNSAFE_REPL=1` opts into a separate-thread Guile REPL and may violate
runtime ownership assumptions. Never enable it in a normal login session.

## Dependencies and provenance

Smithay-derived files retain MIT identifiers and provenance in `NOTICE`; project
code is GPL-3.0-or-later. Guix definitions and Cargo.lock pin the build inputs.
Release archive/offline reproducibility remains Sprint 9 work.

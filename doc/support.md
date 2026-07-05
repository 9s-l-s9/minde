# Support and compatibility

Minde is pre-1.0 and maintained by one person. The normative support policy
is [`../SUPPORT.md`](../SUPPORT.md); APIs, configuration, keymaps and state may
still change without migration layers.

## Evidence-backed scope

The current capability/evidence table is
[`capability-matrix.md`](capability-matrix.md). The bounded application matrix
and exact local commands are in
[`application-testing.md`](application-testing.md). Optional heavyweight
clients are deliberately not part of the default gate.

## Delegated components

The modeline, panel, tray, wallpaper, launcher and lock UI are external tools.
Eww, swaybg and fuzzel have automated layer-shell scenarios. Modern swaylock is
not supported because the session-lock protocol is missing.

## Before reporting

1. Reproduce with the repository default in a nested session.
2. Run the smallest relevant `make check-*` target.
3. Record the Minde and Guix revisions and backend.
4. Create and personally inspect a redacted diagnostic bundle.
5. State whether the problem also occurs without personal startup programs.

Security-sensitive reports follow [`../SECURITY.md`](../SECURITY.md), not a
public issue.

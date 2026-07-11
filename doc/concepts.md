# Window-management concepts

Minde follows the StumpWM model while using Wayland protocol objects.

## Groups, heads, frames, and windows

- A **group** is an ordered workspace with its own windows and layout state.
- A **head** is one usable output rectangle. Each group normally owns a frame
  tree per head; span mode uses one tree over the output union.
- A **frame** is a leaf of a binary split tree. It is a pane, not a window.
- A group's **window list** behaves like an Emacs buffer list. A frame can
  remember several windows while showing one.

Window cycling is group-wide. Pulling selects a hidden window and moves it into
the current frame. Window numbers remain stable until explicitly repacked.

## Manual and dynamic layouts

Manual groups use horizontal/vertical binary splits. Split ratios, gaps,
directional focus, resizing, balancing, dumps, and named layout specifications
all operate on that tree.

Dynamic groups use a master/stack policy per group and head. The newest window
becomes master; map, unmap, group movement and float transitions trigger a
retile. Manual split commands are refused while a dynamic policy owns the
layout.

## Floating and fullscreen windows

Floats remain members of the group window list but are placed above tiled
frames with remembered geometry. `super+left-drag` moves a float and
`super+right-drag` resizes it. Fullscreen temporarily freezes ordinary frame
synchronization; leaving fullscreen restores the tree geometry.

## Outputs and hotplug

Output geometry may have negative origins, vertical arrangement, different
sizes, or gaps. Pointer confinement chooses the nearest output rectangle.
Removing a head adopts its windows into a surviving head. Hardware hotplug is
still an owner-validation item even though deterministic geometry and
simulated-head tests exist.

## Wayland, Xwayland, and layers

Native xdg-shell and Xwayland toplevels enter the same group/frame policy.
Layer-shell surfaces such as swaybg, fuzzel and Eww are arranged independently;
exclusive zones reduce the space available to frame trees. The modeline and
system tray remain external responsibilities.

Modern swaylock uses `ext-session-lock-v1`, which backs the `lock-screen!`
and `suspend!` commands in `(minde session)`. Layer shell is a separate
mechanism and must not be treated as a working lock on its own.

The current support boundary and evidence are tracked in
[`capability-matrix.md`](capability-matrix.md).

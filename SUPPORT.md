# Support policy

minde is currently unreleased and maintained by one person. Support is
best-effort and the API, configuration, keymap, CLI, and persisted state may
change without compatibility aliases before 1.0.

## In scope before 1.0

- Reproducible bugs on the pinned Guix development environment.
- The nested winit backend used by automated tests.
- DRM/udev behavior only on hardware listed as manually verified.
- Portable repository defaults and the documented Scheme API.

## Out of scope

- A built-in modeline, panel, or system tray; use a separate tool such as eww.
- Support for arbitrary distributions or untested GPUs before 1.0.
- Personal programs and host-specific configuration from the maintainer's
  System repository.

For a bug, run the smallest relevant `make check-*` target and include the
revision from `minde --version`, backend, Guix system revision, reproduction
steps, and a minimal configuration. Remove private window titles, clipboard
data, command arguments, and home paths before sharing diagnostics.

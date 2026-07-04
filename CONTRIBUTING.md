# Contributing and testing

minde is preparing its first release. Breaking improvements are welcome,
but public Scheme APIs introduced by the release roadmap should be documented
and tested in the same change.

## Development environment

Enter the repository environment once so Cargo dependencies and native
libraries remain available across smaller checks:

```sh
guix shell -m manifest.scm shellcheck
```

Run independent checks separately (and in parallel from separate terminals if
desired):

```sh
make check-rust
make check-scheme
make check-static
make check-docs
make check-foundation
make check-ui
```

The full local gate, including nested graphical tests, is:

```sh
guix shell -m manifest.scm xorg-server xdotool imagemagick foot xterm \
  shellcheck -- make check-all
```

Use `make check-package` for the offline Guix package build. Hardware/TTY
validation is a separate, explicitly manual gate because it takes control of
a real seat.

## Change expectations

- Keep Rust formatted and free of Clippy warnings.
- Add Scheme or Rust tests at the lowest layer that can catch the regression.
- Keep generated files, logs, and screenshots out of the repository.
- Update CHANGELOG.md for user-visible changes.
- Preserve SPDX headers and NOTICE entries on upstream-derived code.
- Keep repository defaults portable; personal programs and host policy belong
  in the separate System configuration.

By contributing, you agree that your contribution is licensed under the
license declared by the file, or GPL-3.0-or-later when no file-specific SPDX
identifier is present.

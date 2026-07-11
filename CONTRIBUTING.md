# Contributing and testing

minde is preparing its first release. Breaking improvements are welcome,
but public Scheme APIs introduced by the release roadmap should be documented
and tested in the same change.

## Development environment

Enter the repository environment once so Cargo dependencies and native
libraries remain available across smaller checks:

```sh
guix shell -m manifest.scm
```

Normal development and pre-commit verification use one command:

```sh
./check
```

It always runs the same fixed fast gate. `./check --help` lists the optional
focused, integration, and release modes. Granular
`make check-*` targets are retained for debugging and CI, but are not required
knowledge for normal contribution.

The bounded integration gate, including nested graphical tests, is:

```sh
./check --all
```

`./check --release` runs sequentially to bound memory. Hardware/TTY validation
remains explicitly manual because it takes control of a real seat. Run the
larger GUI compatibility matrix in the bounded batches documented in
`doc/application-testing.md`; do not combine every browser and toolkit in one
Guix environment.

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

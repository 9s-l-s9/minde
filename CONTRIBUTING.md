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

The vendored, git-pinned Smithay crate graph is large, so the first
`cargo build`/`cargo check` or rust-analyzer index after a clean checkout is
slow; see `doc/debugging.md#cold-build` for measured numbers and why
sccache/mold are deliberately not part of the default toolchain. For Scheme,
Geiser or another Guile-aware client works against a normal `guile -L
scheme` REPL; see `doc/debugging.md#editor-setup` and `#interactive-scheme-debugging`.

## Continuous integration

Hosted CI runs exactly one script, `scripts/ci`, the same one you can run
locally:

```sh
guix shell -m manifest.scm -- scripts/ci
```

It composes the fast gate (`./check`, which already includes the documentation
and static-analysis gates) and the release-metadata check. On version tags the
hosted workflow additionally runs `scripts/ci --release-artifacts`, which builds
the deterministic source and vendored archives, verifies they are
byte-reproducible, and writes them with `SHA256SUMS` to `build/ci/artifacts`.
No gate logic lives in `.github/workflows/ci.yml`; if a hosted run disagrees
with a local one, the bug is in `scripts/ci`. Nested/e2e, application, demo, and
soak gates stay out of CI because headless runners cannot support them; run them
locally with `./check --all` or the granular `make check-*` targets.

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

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

It always runs the same fixed gate, including Rust unit tests and Clippy.
ShellCheck is required: its absence or a shell lint failure fails the gate.
Documentation link checks require ripgrep and fail on discovery or extraction
errors; a document without links is valid. Generated-document checks leave
source documents untouched and reject stale testing references, including
when the command inventory changes. The static gate exercises these cases,
link-check failures and demo-duration validation with isolated fixtures.
`./check --help` lists the optional
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

`make compile-scheme` reuses unchanged development bytecode. Any Scheme
content change, including a dependency edit or deleted file, rebuilds the
whole cache because Guile can inline across modules. The compiler and driver
also participate in cache invalidation; failed builds retain the previous
complete cache and fail the invoking check. `tests/scheme-cache-test.sh`
exercises this behavior with real Guile compilation in a temporary tree.

## Continuous integration

Hosted development checks use `scripts/ci`, the same entry point you can run
locally:

```sh
guix shell -m manifest.scm -- scripts/ci
```

It composes the development gate (`./check`, which includes documentation and
static analysis) and the release-metadata check. On version tags the
hosted workflow additionally runs `scripts/ci --release-artifacts`, which builds
the deterministic source and vendored archives, verifies they are
byte-reproducible and satisfy the complete locked dependency graph offline,
and writes them with `SHA256SUMS` to `build/ci/artifacts`.
Vendoring uses `--locked`; archive validation resolves dependencies from the
extracted mirror with an empty Cargo home. Rust integration tests reject
incomplete mirrors, wrong crate versions and outdated lockfiles.
No gate logic lives in `.github/workflows/ci.yml`; if a hosted run disagrees
with a local one, the bug is in `scripts/ci`. Nested/e2e, application, demo, and
soak gates stay out of CI because headless runners cannot support them; run them
locally with `./check --all` or the granular `make check-*` targets.

Tagged source releases additionally use `scripts/ci-release` to require a
tag matching Cargo's version on `origin/main`. The helper checks the checkout
and event revisions again before publishing and marks candidate versions as
prereleases. Its tests use temporary Git histories and a mocked publisher.

The Pages workflow builds `site/` with the same verified Guix bootstrap and
pinned channel revision. It pins actions by commit, filters TLS roots before
checkout and deployment, and keeps Pages/OIDC write permissions in the deploy
job. The build job has only contents and Pages read access, with automatic
Pages enablement disabled. Changes to its bootstrap or channel pin also
trigger a site rebuild.

## Change expectations

- Use Rust and Guile Scheme for implementations and tests, with shell scripts
  for orchestration. Do not add Python scripts or interpreter dependencies.
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

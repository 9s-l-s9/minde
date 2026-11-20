# Packaging and releasing

The release-candidate version is `1.0.0-rc1`. Cargo, the three Guix packages, README,
changelog, CLI, Smithay vendor configuration, and documented schema versions
are checked together by `make check-docs`.

Status and diagnostic schemas: `1`.
Persistent state formats: layouts `1`, placement rules `2`.

## Tagged source releases

The source-archive CI job accepts only `v<Cargo version>` tags whose commit
is on `origin/main` and matches both the checkout and the triggering event.
It fetches full history and runs `scripts/ci-release --check` before building.
Hosted `scripts/ci --release-artifacts` runs the same validation before its
gates; local archive checks can still run without a tag.

`scripts/ci-release --publish` repeats the checks and publishes only the two
archives for that version and their `SHA256SUMS`. Versions with a
prerelease suffix, such as `1.0.0-rc1`, use GitHub's prerelease flag. Stable
versions remain normal releases, including versions with build metadata.
This source-release policy is separate from the Guix binary builds below.

## GitHub-built Guix binaries

`.github/workflows/guix-binary.yml` builds the complete Guix package on pushes
to `main`, and can also be started manually on `main`. It publishes a prerelease
named `guix-<full commit>`, containing the signed runtime closure, metadata,
the build's public signing key, and GitHub provenance. It includes Xwayland,
which is a propagated runtime dependency. Build dependencies are not exported.
The job reserves its commit tag before building; a newer push supersedes an
unfinished build. This avoids GitHub's restriction on job tokens tagging older
commits after workflow changes. A failed build can leave a tag without a release.

Binary download retention keeps the five newest mutable, published CI
prereleases, ordered by publication time. The current build and any active
workflow runs are protected even if they fall outside those five. Version
releases, drafts, immutable releases, manually published releases and tags
whose targets differ from their embedded commit are preserved.

`scripts/ci-binary-retention --apply` runs before reserving the current build's
tag and again after publication. It removes older eligible releases before
deleting their tags. A reservation without a release is removed only when a
completed main-workflow run identifies its commit and no active run needs it;
unknown reservations remain untouched. Thus failed or cancelled builds are
cleaned up on a later run, including when that later build itself fails.
Every API page must load and validate, and each candidate is rechecked before
deletion. The workflow's shared concurrency group serializes these operations.
The helper uses the workflow's `contents: write` and `actions: read` permissions.

Run `scripts/ci-binary-retention --plan` for a read-only JSON deletion plan.
Applying it is restricted to the canonical binary workflow on `main`. Its
stateful API fixtures run in `make check-static` and cover pagination, protected
references, concurrent changes, failed requests and retry after partial deletion.
The relevant GitHub API contracts are the [release list](https://docs.github.com/en/rest/releases/releases#list-releases),
[matching references](https://docs.github.com/en/rest/git/refs#list-matching-references)
and [workflow runs](https://docs.github.com/en/rest/actions/workflow-runs#list-workflow-runs-for-a-workflow).

Older binary download URLs expire under this policy. Existing imported closures
remain protected by their local Guix garbage-collection roots. A committed
binary lock file does not extend download retention: importing on a new machine
after its build expires requires selecting an available verified build or
building from source.

The builder uses the authenticated revision in `guix/ci-channels.scm` and an
upstream-signed Guix 1.5.0 bootstrap whose SHA-256 is pinned in
`scripts/ci-bootstrap-guix`. Cargo vendors the locked dependency graph, then
Guix builds and tests the archive in its isolated build environment. Neither a
developer's `target/` cache nor a local checkout is used for the installed binary.

To consume a binary release, select the full source commit. Verify that the
GitHub provenance names that commit and this repository's `guix-binary.yml`
workflow on `main`, running on a GitHub-hosted runner. The attested
`binary.scm` records the Minde and Xwayland store paths and the SHA-256
digests of the archive, closure list and ephemeral signing key. Verify those
digests before importing the archive, and authorize the key for that import
only. Pin the commit and digests in the consuming configuration so replacing
a release asset cannot silently change an installation.

CI exercises the Guix import with a temporary key authorization. It checks
that an unauthorized archive is rejected, that a signed store item can be
restored, and that the daemon's global trust list is preserved.

The binary build excludes `emSign ECC Root CA - G3` and the older
`AAA Certificate Services` root by certificate fingerprint, covering both trust
paths for the compromised eMudhra intermediate keys. TLS verification stays
enabled. This is scoped to these workflows and downloads; it does not update
the machine's general-purpose trust store. See the
[Mozilla incident report](https://bugzilla.mozilla.org/show_bug.cgi?id=2083449).

## Package entry points

`guix.scm` packages the current checkout for local development. It deliberately
includes the ignored `vendor/` tree and excludes VCS, agent, cache, target, and
generated-media paths. Run:

```sh
make check-package
```

`guix/release.scm` is the public, archive-only entry point. It refuses to
evaluate without `MINDE_SOURCE_ARCHIVE`, `MINDE_VERSION`, and
`MINDE_BUILD_REVISION`; this prevents a public build from silently reading
the maintainer's checkout.

Both packages install:

- `minde`, the session wrapper, and the three control helpers;
- the complete runtime Scheme tree and reusable Guile modules;
- the portable configuration and Wayland session entry;
- README, changelog, policies, provenance, licenses, guides, generated API and
  keymap references, and the offline HTML manual.

## Reproducible archives

The normal source archive excludes `vendor/`. The second archive includes the
current ignored vendor tree and is suitable for offline Cargo and Guix builds:

```sh
make check-release-archives
make release-archives VERSION=1.0.0-rc1
```

Archive order, owner/group, timestamps, tar format, and gzip metadata are
normalized. The check creates each archive twice, compares the bytes, rejects
workspace-only paths, and resolves the complete locked Cargo graph offline.
Resolution uses the extracted vendor tree and an empty Cargo home, so a host
cache cannot hide missing or incompatible crates. A regression test in
`cargo test` exercises complete, incomplete, version-mismatched and unlocked
fixture archives without network access.
Either archive build failing, either byte comparison differing, or the final
verification failing stops `make check-release-archives` with an error.
Changing dependencies requires refreshing the ignored mirror first:

```sh
guix shell -m manifest.scm -- cargo vendor --locked vendor
```

Do not commit `vendor/`.

## Full local release

First update every checked version field and commit all intended changes. Then
verify the release gates from the development checkout:

```sh
./check --release
```

This checks the bounded integration suite, regenerates and validates scripted
videos, builds the local Guix package, and compares deterministic archives.
It does not publish or create release notes. Run the actual release from a
clean worktree:

```sh
make release VERSION=1.0.0-rc1
```

Keep exactly one `## Unreleased` section in `CHANGELOG.md`, with actual change
text. Blank sections, subsection headings, and the target-version line alone
do not count as changes. The runner validates and captures this section before
entering its environment or replacing previous output; missing or ambiguous
sections stop the release before any build gates run.

The release runner executes gates sequentially to bound memory use. Its normal
gate set is `check`, nested E2E, the bounded core application matrix,
documentation, scripted video generation and validation, and the local Guix
package. It automatically enters a Guix shell containing exactly those build,
core-application, and video tools. The heavier toolkit/browser shards remain
opt-in. The runner then
creates and validates both archives, builds the archive-only Guix package,
inspects installed content, and writes `RELEASE-NOTES.md` plus `SHA256SUMS`
under `build/release/VERSION/`.

`MINDE_RELEASE_GATES` exists only to shorten development of the release
script. A published artifact must use the default gate set.

## Owner verification of release archives

No system or Home reconfiguration is required for archive verification. Use a
clean temporary checkout so personal untracked files cannot weaken the clean
tree check:

```sh
make check-package
make check-release-archives
make release VERSION=1.0.0-rc1
```

Then disconnect networking, extract the vendored archive, and run:

```sh
mkdir -p .cargo
cp guix/cargo-config.toml .cargo/config.toml
cargo metadata --locked --offline --format-version 1
```

Inspect the Guix store path printed during the release. Confirm that
`share/wayland-sessions/minde.desktop`, both Scheme install roots, the
portable configuration, all guides/licenses, and `doc/generated/manual.html`
exist. Run `bin/minde --version` and confirm it reports the release version
and commit. Search installed text for the temporary checkout path; there must
be no match.

To build the vendored archive from the checkout that produced it:

```sh
MINDE_SOURCE_ARCHIVE="$PWD/build/release/1.0.0-rc1/minde-1.0.0-rc1-vendored.tar.gz" \
MINDE_VERSION=1.0.0-rc1 MINDE_BUILD_REVISION="$(git rev-parse HEAD)" \
guix build -f guix/release.scm
```

For a login test, add the release package to the operating system's packages
and select Minde in the SDDM session menu. Keep the prior Guix generation
available for rollback.

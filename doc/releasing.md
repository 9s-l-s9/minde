# Packaging and releasing

The release-candidate version is `1.0.0-rc1`. Cargo, the three Guix packages, README,
changelog, CLI, Smithay vendor configuration, and documented schema versions
are checked together by `make check-docs`.

Status and diagnostic schemas: `1`.
Persistent state formats: `1`.

## GitHub-built Guix binaries

`.github/workflows/guix-binary.yml` builds the complete Guix package on pushes
to `main`, and can also be started manually on `main`. It publishes a prerelease
named `guix-<full commit>`, containing the signed runtime closure, metadata,
the build's public signing key, and GitHub provenance. It includes Xwayland,
which is a propagated runtime dependency. Build dependencies are not exported.

The builder uses the authenticated revision in `guix/ci-channels.scm` and an
upstream-signed Guix 1.5.0 bootstrap whose SHA-256 is pinned in
`scripts/ci-bootstrap-guix`. Cargo vendors the locked dependency graph, then
Guix builds and tests the archive in its isolated build environment. Neither a
developer's `target/` cache nor a local checkout is used for the installed binary.

The personal System repository consumes these builds with:

```sh
cd ../System
make minde-install COMMIT=<full commit from a successful guix-binary run>
make reconfigure-system
make reconfigure-home
```

The import requires `guix`, `gh`, `xz`, `unshare`/`mount`, internet access, and
`sudo` on a machine with a root-owned Guix store (as on these System configurations). Commit the
resulting `minde-binary.lock.scm` with the System configuration. On another
`x86_64-linux` PC using that configuration, `make minde-install` imports the
same pinned build; no Minde source checkout or Rust compiler is needed.

The importer verifies GitHub's signature against the exact repository,
workflow, `main` ref, and source commit, and rejects self-hosted attestations.
The attested metadata pins the archive, closure list, and Guix public key by
SHA-256. The privileged import copies the inputs to a private directory and
verifies them again before using Guix's signature-checking NAR importer.
The build key is authorized only in that process's temporary Guix configuration;
it is never added to `/etc/guix/acl`. No long-lived signing secret is stored in
GitHub. The import runs in a private mount namespace, preserving the system's
read-only store mount. CI tests rejection of an unauthorized archive and the
successful restoration of a signed store item before publishing. Installed
outputs are retained under `/var/guix/gcroots/minde-binary/`.

The System selector defaults to the imported binary and fails with instructions
if it is missing. Its local derivations only create symlinks to the imported
Minde and Xwayland outputs. Updating unrelated Guix channels or editing the
Minde checkout therefore does not compile Minde. `MINDE_BUILD_LOCAL=1` explicitly
opts back into a source build; the RC archive overrides below remain available.

The build and import paths exclude `emSign ECC Root CA - G3` and the older
`AAA Certificate Services` root by certificate fingerprint, covering both trust
paths for the compromised eMudhra intermediate keys. TLS verification stays
enabled. This is scoped to these workflows and downloads; it does not update
the machine's general-purpose trust store. See the
[Mozilla incident report](https://bugzilla.mozilla.org/show_bug.cgi?id=2083449).

The GitHub Release itself is not an immutable trust anchor: repository admins
can modify assets. The importer rejects changed metadata for an already pinned
commit, and changing the binary requires selecting and reviewing a new commit.

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
Changing dependencies requires refreshing the ignored mirror first:

```sh
guix shell -m manifest.scm -- cargo vendor vendor
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
cargo metadata --locked --offline --no-deps --format-version 1
```

Inspect the Guix store path printed during the release. Confirm that
`share/wayland-sessions/minde.desktop`, both Scheme install roots, the
portable configuration, all guides/licenses, and `doc/generated/manual.html`
exist. Run `bin/minde --version` and confirm it reports the release version
and commit. Search installed text for the temporary checkout path; there must
be no match.

To evaluate the personal System/Home configuration against the RC archive
while retaining the normal checkout fallback:

```sh
export MINDE_RC_ARCHIVE="$PWD/build/release/1.0.0-rc1/minde-1.0.0-rc1-vendored.tar.gz"
export MINDE_RC_REVISION="$(git rev-parse HEAD)"
make -C ~/Projects/System qa-home-samuel qa-system-x1 qa-system-t450s
```

The shared `~/Projects/System/minde-package.scm` selector uses the archive
only when both variables are present. Without them, existing reconfiguration
continues to use the checkout package. Preserve both variables explicitly if
you later invoke a privileged reconfiguration for the live SDDM test.

To test the SDDM entry in a real login, install the package through the System
configuration, reconfigure, and restart into a fresh session. Keep the prior
Guix generation and StumpWM session available for rollback. That live-login
step and switching `~/Projects/System` from the checkout to the actual RC
artifact remain pending until performed by the owner.

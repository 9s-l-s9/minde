SHELL := /bin/sh

SCHEME_TESTS := \
	tests/foundation-test.scm \
	tests/property-test.scm \
	tests/ui-prompt-test.scm \
	tests/config-test.scm \
	tests/status-test.scm \
	tests/portable-keymap-test.scm \
	tests/input-test.scm \
	tests/frames-test.scm \
	tests/groups-test.scm \
	tests/session-test.scm \
	tests/layouts-test.scm \
	tests/next-pull-test.scm \
	tests/heads-test.scm \
	tests/floats-test.scm \
	tests/menu-test.scm \
	tests/winmgmt-test.scm \
	tests/placement-test.scm \
	tests/dynamic-test.scm

.PHONY: check check-tools check-rust check-cli check-scheme check-api check-config check-keymaps check-foundation check-ui check-static check-e2e check-stress check-soak \
	check-apps check-apps-all check-apps-core check-apps-toolkits check-apps-desktop check-apps-layer check-apps-strict check-docs check-package check-all check-hardware demos \
	docs check-generated-docs check-demos check-foundation-package check-ui-package \
	release-archives check-release-archives release clean-test-output

check: check-tools check-rust check-cli check-static check-scheme check-api check-config check-keymaps

check-tools:
	@command -v cargo >/dev/null 2>&1 || { \
		echo "error: cargo is required; enter: guix shell -m manifest.scm" >&2; exit 127; }
	@command -v guile >/dev/null 2>&1 || { \
		echo "error: guile is required; enter: guix shell -m manifest.scm" >&2; exit 127; }

check-rust:
	cargo fmt --all -- --check
	cargo build --locked
	cargo test --locked
	cargo clippy --locked --all-targets -- -D warnings

check-cli: check-rust
	sh tests/check-cli.sh target/debug/minde

check-static:
	sh tests/lint-borrows.sh
	@if command -v shellcheck >/dev/null 2>&1; then \
		shellcheck -x check debug-tty.sh tests/*.sh tests/lib/*.sh \
			scripts/capture-demos scripts/generate-docs scripts/generate-testing-reference \
			scripts/generate-packaging-reference \
			scripts/minde-cmd \
			scripts/minde-msg scripts/mindectl \
			scripts/create-release-archives scripts/check-guix-package \
			scripts/hardware-report scripts/release; \
	else \
		echo "note: shellcheck unavailable; static shell analysis skipped"; \
	fi

check-scheme:
	@set -eu; for test in $(SCHEME_TESTS); do \
		echo "== $$test =="; \
		guile --no-auto-compile -L scheme "$$test"; \
	done

check-api:
	guile --no-auto-compile -L scheme tests/api-test.scm

check-config:
	guile --no-auto-compile -L scheme tests/config-test.scm
	sh scripts/mindectl check-config scheme/default-config.scm

check-keymaps:
	guile --no-auto-compile -L scheme tests/portable-keymap-test.scm
	sh tests/check-portable-defaults.sh

check-foundation:
	env -u DISPLAY -u WAYLAND_DISPLAY \
		guile --no-auto-compile -L scheme tests/foundation-test.scm

check-ui:
	env -u DISPLAY -u WAYLAND_DISPLAY \
		guile --no-auto-compile -L scheme tests/ui-prompt-test.scm
	env -u DISPLAY -u WAYLAND_DISPLAY \
		guile --no-auto-compile -L scheme tests/menu-test.scm

check-e2e:
	@command -v Xvfb >/dev/null 2>&1 || { \
		echo "error: Xvfb is required; add xorg-server to the Guix shell" >&2; exit 127; }
	@command -v xdotool >/dev/null 2>&1 || { \
		echo "error: xdotool is required; add it to the Guix shell" >&2; exit 127; }
	@command -v import >/dev/null 2>&1 || { \
		echo "error: ImageMagick 'import' is required" >&2; exit 127; }
	@command -v jq >/dev/null 2>&1 || { \
		echo "error: jq is required by the portable nested scenario" >&2; exit 127; }
	@command -v foot >/dev/null 2>&1 || { \
		echo "error: foot is required by the run-prompt and portable scenarios" >&2; exit 127; }
	@command -v grim >/dev/null 2>&1 || { \
		echo "error: grim is required by the screen-capture scenario" >&2; exit 127; }
	@command -v identify >/dev/null 2>&1 || { \
		echo "error: ImageMagick 'identify' is required by the screen-capture scenario" >&2; exit 127; }
	@command -v wl-paste >/dev/null 2>&1 || { \
		echo "error: wl-clipboard is required by the clipboard scenario" >&2; exit 127; }
	@command -v wayland-info >/dev/null 2>&1 || { \
		echo "error: wayland-utils (wayland-info) is required by the clipboard/foreign-toplevel scenarios" >&2; exit 127; }
	@command -v wlr-randr >/dev/null 2>&1 || { \
		echo "error: wlr-randr is required by the output-management scenario" >&2; exit 127; }
	@command -v swayidle >/dev/null 2>&1 || { \
		echo "error: swayidle is required by the idle scenario" >&2; exit 127; }
	sh tests/e2e.sh
	sh tests/portable-e2e.sh
	sh tests/screencapture-e2e.sh
	sh tests/clipboard-e2e.sh
	sh tests/foreign-toplevel-e2e.sh
	sh tests/output-management-e2e.sh
	sh tests/pointer-constraints-e2e.sh
	sh tests/fractional-scale-e2e.sh
	sh tests/idle-e2e.sh
	sh tests/cursor-shape-e2e.sh
	sh tests/text-input-e2e.sh

check-stress:
	@command -v Xvfb >/dev/null 2>&1 || { echo "error: Xvfb is required" >&2; exit 127; }
	@command -v xdotool >/dev/null 2>&1 || { echo "error: xdotool is required" >&2; exit 127; }
	@command -v import >/dev/null 2>&1 || { echo "error: ImageMagick 'import' is required" >&2; exit 127; }
	MINDE_E2E_STRESS=1 sh tests/e2e.sh

check-apps: check-apps-core

check-apps-all:
	sh tests/applications.sh

check-apps-core:
	MINDE_APPS_FILTER=foot,wl-clipboard,xterm sh tests/applications.sh

check-apps-toolkits:
	MINDE_APPS_FILTER=gtk3,gtk4,qt5,qt6,sdl2 sh tests/applications.sh

check-apps-desktop:
	MINDE_APPS_FILTER=electron,chromium,firefox,emacs-pgtk sh tests/applications.sh

check-apps-layer:
	MINDE_APPS_FILTER=swaybg,fuzzel,swaylock,eww sh tests/applications.sh

# Release/CI gate: every matrix entry must be installed and pass. The normal
# target records explicit skips so contributors can run a useful subset.
check-apps-strict:
	MINDE_APPS_STRICT=1 sh tests/applications.sh

check-soak:
	sh tests/soak.sh

docs:
	sh scripts/generate-docs

check-generated-docs:
	sh tests/check-generated-docs.sh

check-docs: check-generated-docs
	@test -s README.md
	@test -s PLAN.md
	@test -s doc/release-roadmap.md
	@test -s doc/api.md
	@test -s doc/tutorial.md
	@test -s doc/configuration.md
	@test -s doc/concepts.md
	@test -s doc/keybindings.md
	@test -s doc/ipc-eww.md
	@test -s doc/debugging.md
	@test -s doc/testing.md
	@test -s doc/architecture.md
	@test -s doc/security.md
	@test -s doc/hardware-validation.md
	@test -s doc/support.md
	@test -s doc/diagnostics.md
	@test -s doc/application-testing.md
	@test -s doc/demonstrations.md
	@test -s doc/releasing.md
	@test -s doc/generated/api-reference.md
	@test -s doc/generated/keybindings.md
	@test -s doc/generated/demo-manifest.json
	@test -s doc/generated/manual.html
	@test -s doc/reusable-packages.md
	@test -s UNEXPECTED.md
	sh tests/check-doc-links.sh
	sh tests/check-release-metadata.sh

check-package:
	@command -v guix >/dev/null 2>&1 || { echo "error: guix is required" >&2; exit 127; }
	sh scripts/check-guix-package guix.scm

release-archives:
	@test -n "$(VERSION)" || { echo "error: VERSION is required" >&2; exit 2; }
	sh scripts/create-release-archives "$(VERSION)"

check-release-archives:
	@version="$(VERSION)"; \
	if [ -z "$$version" ]; then \
		version=$$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1); \
	fi; \
	rm -rf build/release-check; \
	sh scripts/create-release-archives "$$version" build/release-check/one >/dev/null; \
	sh scripts/create-release-archives "$$version" build/release-check/two >/dev/null; \
	cmp build/release-check/one/minde-$$version.tar.gz \
		build/release-check/two/minde-$$version.tar.gz; \
	cmp build/release-check/one/minde-$$version-vendored.tar.gz \
		build/release-check/two/minde-$$version-vendored.tar.gz; \
	guix shell -m manifest.scm -- sh tests/check-release-archives.sh "$$version" \
		build/release-check/one/minde-$$version.tar.gz \
		build/release-check/one/minde-$$version-vendored.tar.gz

release:
	@test -n "$(VERSION)" || { echo "error: VERSION is required" >&2; exit 2; }
	sh scripts/release "$(VERSION)"

check-foundation-package:
	guix build -f guix/foundation.scm

check-ui-package:
	guix build -f guix/ui.scm

check-all: check check-e2e check-apps check-docs

check-hardware:
	@report=$$(sh scripts/hardware-report); \
	echo "hardware snapshot: $$report"; \
	echo "complete it from a spare VT using doc/hardware-validation.md"

demos:
	sh scripts/capture-demos

check-demos: check-generated-docs
	sh tests/check-demos.sh

clean-test-output:
	rm -rf /tmp/minde-e2e

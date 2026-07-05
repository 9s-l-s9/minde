SHELL := /bin/sh

SCHEME_TESTS := \
	tests/foundation-test.scm \
	tests/ui-prompt-test.scm \
	tests/config-test.scm \
	tests/status-test.scm \
	tests/portable-keymap-test.scm \
	tests/input-test.scm \
	tests/frames-test.scm \
	tests/groups-test.scm \
	tests/layouts-test.scm \
	tests/next-pull-test.scm \
	tests/heads-test.scm \
	tests/floats-test.scm \
	tests/menu-test.scm \
	tests/winmgmt-test.scm \
	tests/placement-test.scm \
	tests/dynamic-test.scm

.PHONY: check check-tools check-rust check-cli check-scheme check-api check-config check-keymaps check-foundation check-ui check-static check-e2e check-stress \
	check-apps check-docs check-package check-all check-hardware demos \
	check-foundation-package check-ui-package clean-test-output

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
		shellcheck check debug-tty.sh tests/*.sh scripts/minde-cmd scripts/minde-msg scripts/mindectl; \
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
	sh tests/e2e.sh

check-stress:
	@command -v Xvfb >/dev/null 2>&1 || { echo "error: Xvfb is required" >&2; exit 127; }
	@command -v xdotool >/dev/null 2>&1 || { echo "error: xdotool is required" >&2; exit 127; }
	@command -v import >/dev/null 2>&1 || { echo "error: ImageMagick 'import' is required" >&2; exit 127; }
	MINDE_E2E_STRESS=1 sh tests/e2e.sh

# Sprint 7 expands this target into the full toolkit matrix. Until then, the
# existing e2e suite is the application smoke test (foot plus optional xterm).
check-apps: check-e2e

check-docs:
	@test -s README.md
	@test -s PLAN.md
	@test -s doc/release-roadmap.md
	@test -s doc/api.md
	@test -s doc/diagnostics.md
	@test -s doc/reusable-packages.md
	@test -s UNEXPECTED.md
	sh tests/check-release-metadata.sh

check-package:
	@command -v guix >/dev/null 2>&1 || { echo "error: guix is required" >&2; exit 127; }
	guix build -f guix.scm

check-foundation-package:
	guix build -f guix/foundation.scm

check-ui-package:
	guix build -f guix/ui.scm

check-all: check check-apps check-docs

check-hardware:
	@echo "Run ./debug-tty.sh from a spare VT; see doc/hardware-validation.md when Sprint 7 adds the guided checklist."

demos:
	@echo "Demo capture is introduced in Sprint 8."

clean-test-output:
	rm -rf /tmp/minde-e2e

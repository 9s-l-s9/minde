#!/bin/sh
set -eu

command -v rg >/dev/null 2>&1 || {
    echo "error: rg (ripgrep) is required; enter: guix shell -m manifest.scm" >&2
    exit 127
}

set -- scheme/init.scm scheme/default-config.scm guix.scm
pattern='Projects/System|Projects/images|Projects/WorkingMemory|de.*bone|swaybg|swaylock|emacsclient|ASK_AI_SYSTEM'

if rg -n "$pattern" "$@"; then
    echo "error: personal or machine-specific policy remains in repository defaults" >&2
    exit 1
fi

echo "portable defaults: ok"

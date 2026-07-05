#!/bin/sh
set -eu

set -- scheme/init.scm scheme/default-config.scm guix.scm
pattern='Projects/System|Projects/images|Projects/WorkingMemory|de.*bone|swaybg|swaylock|emacsclient|ASK_AI_SYSTEM'

if rg -n "$pattern" "$@"; then
    echo "error: personal or machine-specific policy remains in repository defaults" >&2
    exit 1
fi

echo "portable defaults: ok"

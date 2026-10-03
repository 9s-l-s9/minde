# Control API validation evidence

`nested-metrics.json` records the isolated native Wayland scenario run on
2026-10-03 with `guix shell -m manifest.scm -- sh tests/control-e2e.sh`.
It uses two synthetic `foot` windows with duplicate titles in different
groups. The assertions cover discovery, cross-group focus, explicit floating
state, deduplication, conflicting IDs, stale requests, screenshot success
and asynchronous file-write failure, movement, vanished targets, and close
completion. The deterministic selector also performs a dry run, abstains on
duplicate titles, and executes one allowlisted action through the same API.

The 44 measured CLI subprocesses include startup, transport, and response
parsing overhead: p50 141.0 ms, p95 185.9 ms, maximum response 4,846 bytes.
Three selector subprocess timings are recorded separately. These are one
run on this development host, not performance guarantees or model benchmarks.
Regenerate with the command above; it writes fresh results to
`/tmp/minde-control-e2e/results.json` and does not overwrite this record.

A separate fresh reviewer trial on 2026-10-02 used only `mindectl` help,
discovery, and typed calls, without repository source, documentation, or
raw `eval`. It completed cross-group focus, fullscreen on/off, screenshot,
receipt observation, and verification: 15 CLI calls, 9 tool calls, zero CLI
failures or wrong targets, about 6.5 seconds inside CLI subprocesses. That
trial preceded the final committed-geometry refinement; the nested scenario
above checks the final geometry implementation. The reviewer noted that
screenshot help confused automation tokens with request IDs; the help now
names the receipt used by `wait`. Compound action values intentionally retain
the documented `$scheme` JSON representation.

Multioutput target selection is checked by native geometry regressions, and
cross-head policy by Scheme tests. The nested backend supplies one output;
this record does not establish physical multi-monitor behavior. Live Jev and
local-model quality/latency measurements remain deferred. See the separate
[classifier evidence](../control-agent/baseline-metrics.json).

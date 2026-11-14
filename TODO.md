# Open design decisions and measurements

Track defects in the [issue tracker](https://github.com/9s-l-s9/minde/issues).
[PLAN.md](PLAN.md) and the [agent API plan](doc/agent-api-plan.md) describe
future features. This file keeps unresolved choices and profiling questions;
implemented work and earlier audit notes remain in Git history.

## Design decisions

- **Return an explicitly positioned output to automatic layout.** Decide the
  public `configure-output! #:position 'auto` contract. Failed configurations
  already restore the previous explicit-position flag; the missing API is a
  separate feature, not an outstanding rollback defect.
- **CLI module selection.** Decide whether `MINDE_SCHEME_DIR` should override
  modules beside `mindectl`, as it does for the compositor. The CLI currently
  checks adjacent checkout/installed modules before that environment variable.
  Keep installed clients and their protocol modules compatible when changing
  this precedence.
- **CLI startup and persistent requests.** Measure Guile startup separately
  from compositor evaluation before choosing a Rust helper or persistent
  transport. Ordinary commands still start a Guile process; subscriptions and
  the interactive client already retain one. Any framing change must preserve
  the documented IPC contract and bounded observation deadlines.
- **Release API fingerprint.** Consider hashing semantic API data rather than
  the generated reference Markdown, whose source line numbers change after
  unrelated edits. Keep intentional public API changes visible to the release
  gate; changing the hash input needs its own compatibility decision.

## Measure before changing

- Complete the remaining reference-machine checks in
  [hardware validation](doc/hardware-validation.md), including multi-output
  presentation and capture/recording costs on real GPUs. Nested pixel checks
  and software-renderer timings do not establish physical display latency.
- Profile scene construction and callback selection with representative window
  and output counts. The final scene vectors and per-window
  `Space::element_geometry` lookups remain allocation/lookup candidates.
  Reusing renderer-typed vectors requires a lifetime-safe ownership design;
  cached bounding boxes cannot replace current window geometry indiscriminately.
- Include status publication and real client/event-loop work when measuring
  policy latency. Synthetic focus benchmarks exclude those costs. Measure the
  remaining status-body/fingerprint rebuilds and activation walks before adding
  mutation counters or more caches.
- Separate Scheme startup, device probing and application startup in a cold
  login profile before overlapping initialization. Backend setup invokes Scheme
  hooks synchronously, so moving it ahead of policy loading requires a lifecycle
  change. Saved layouts and rules must still load when users replace startup hooks.

Reproduction commands, measured results and their limits live in
[performance measurements](doc/performance.md). Record new evidence there
before promoting a profiling question into implementation work.

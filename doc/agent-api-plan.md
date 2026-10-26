# Plan: Scheme API, REPL, and agent experience

Date: 2026-10-02. Status: implemented; validation updated 2026-10-03.
Live Jev evaluation is deferred by the user's explicit choice.

The operational interface is documented in [agent control](agent-control.md).
The phase descriptions below retain the agreed requirements. The recorded
evidence and remaining measurement limits are listed separately below.

## Implementation status

| Phase | Implemented surface | Validation boundary |
|---|---|---|
| Dependable replies | Type-based bounded serialization, explicit unspecified values, failure stages, complete machine envelopes | Serializer regressions and CLI transport tests cover supported data and unknown outcomes; final gate results follow below |
| Discoverable operations | Typed command schemas, defaults/constraints, concise discovery, detailed action help, user-defined commands | Registry and introspection tests; generated documentation must pass the drift gate |
| Desktop and receipts | Desktop-wide snapshots, session/revision preconditions, typed actions, bounded receipts, asynchronous observation | Scheme policy tests plus isolated nested scenarios; final scenario results follow below |
| CLI and REPL | Inspection/call/request/wait, explicit JSON projection, sequenced watch recovery, multiline remote REPL with completion/history | Client contracts and Unix-socket integration tests; final gate results follow below |
| Classifier adapter | Deterministic baseline, optional Jev provider, loopback selector transport, finite candidates and shared executor | Offline provider stubs and a synthetic baseline only; no model-quality, calibration, or live Jev claim |

Primary additions are `(minde control)`, `(minde control-client)`,
`(minde control-json)`, and `scripts/minde-agent`. Receipt retention is the
last 256 allocations. The event journal has both a 128-event and a 16 KiB
bound; gaps require a new snapshot. The operational result is an alist
inside the existing `(ok VALUE)` envelope, rather than the illustrative
tagged `(result ...)` expression later in this plan.

The synthetic classifier corpus and recorded deterministic metrics are in
[`tests/fixtures/control-agent`](../tests/fixtures/control-agent/corpus.json).
Its ten expected results include five selections and five abstentions, with
zero wrong targets. Timings are selector-only, not desktop end-to-end or
live-provider performance. Credentials were not used and no model was
downloaded. The user explicitly deferred live Jev evaluation and requested
research into local alternatives. GLiClass and GLiNER2 were checked against
official model cards and repositories; both can run locally through separate
libraries, but neither has turnkey model serving or evaluated Minde quality
in this implementation. See the guide's
[local-model section](agent-control.md#local-models).

## Validation evidence

- Rust formatting, build, all 89 non-ignored tests, and Clippy passed. The two
  ignored tests are existing opt-in measurements. Native regressions include
  journal replay/eviction and targeted output selection with negative origins,
  spanning windows, and a pointer on another output.
- The full Scheme/API/configuration/keymap suites, static checks, and generated
  documentation/link/release-metadata gates passed. The control suite has
  158 checks, including real Scheme policy effects, malformed custom results,
  terminal validation receipts, zero window IDs, hooks changing targets,
  multiple heads, lock redaction, and native geometry revision invalidation.
- Fourteen Unix-socket/PTY client tests cover replies, deadlines, multiline
  input, history/completion, event gaps, restart, and reconnect. Eighteen
  offline selector tests cover provider failures and the execution boundary.
- The [recorded nested scenario](../tests/fixtures/control-api/README.md)
  passed with two real Wayland clients: 44 CLI calls, p50 141.0 ms, p95
  185.9 ms, and maximum response size 4,846 bytes. It checks actual focus,
  floating state, movement, close, screenshot dimensions and write failure,
  stale requests, deduplication, and live deterministic adapter integration.
- A fresh agent completed discover/inspect/act/verify using only public CLI
  output, without source, docs, or raw `eval`: 15 CLI calls and zero wrong
  targets or CLI failures. Its evidence boundary is documented with the
  nested metrics. Screenshot help was clarified from its feedback.
- Guix package evaluation succeeds, and the package definition installs the Python helper
  and new Scheme modules. A full package build/install and physical
  multi-monitor run were not performed. Multioutput selection has native
  geometry tests; nested rendering has one output.

Run `./check`, `make check-rust`, and `make check-control-e2e` inside
`guix shell -m manifest.scm` to reproduce the development gates. Keep the
Guile client for this version: its measured startup/round-trip cost is
recorded above, and a second protocol or compiled helper is not needed for
the demonstrated interaction loop. Classifier activation remains explicit,
with no live-model confidence calibration claimed.

## Agreed direction

Improve the coding-agent REPL/API experience first, then add support for
Jev or similar classifiers through the same control surface. This ordering
was explicitly selected by the user. Keep Scheme as the native interface.

The interaction to optimize is **discover → inspect → act → verify**.
An agent should be able to find an operation, understand its arguments,
identify its target, execute it, and verify its outcome without reading
Minde's implementation or parsing human-oriented output.

This plan refines the broader [observability roadmap](../PLAN.md). It
supersedes that roadmap's overlapping API priorities and its proposal to
wait for screenshots inside the compositor. The other roadmap work remains
useful, especially ownership and retraction once agents install temporary
programs.

## Evidence and starting point

The planning investigation inspected the current implementation and ran
the existing IPC reply and API introspection tests. Both passed. No live
Jev calls or compositor performance benchmarks were performed.

| Existing surface | Evidence | Gap to address |
|---|---|---|
| Readable Scheme replies | [ipc-reply.scm](../scheme/ipc-reply.scm) | Serialization can misreport a successful evaluation as an error |
| Runtime discovery and generated catalog | [api-introspect.scm](../scheme/api-introspect.scm), [catalog generator](../scripts/generate-api-catalog.scm) | Need structured argument types, results, effects, and examples |
| Command metadata and invocation | [commands.scm](../scheme/minde/commands.scm), [command-catalog.scm](../scheme/minde/command-catalog.scm) | Invocation checks existence and arity; the 19 built-ins all take zero arguments |
| Versioned state snapshot | [status.scm](../scheme/minde/status.scm) | Designed mainly for bars; lacks a complete inventory of windows across groups |
| Window queries and focus operations | [windows.scm](../scheme/minde/windows.scm), [frame implementation](../scheme/minde/compositor/frames.scm) | `all-window-ids` and `focus-window-by-id!` are scoped to the active group |
| Event stream and automation tokens | [event-stream.scm](../scheme/event-stream.scm), [IPC documentation](ipc-eww.md) | Need consistent completion semantics and recovery after missed events |
| Main-thread control socket | [ipc.rs](../src/ipc.rs) | Synchronous evaluation cannot be interrupted by its event-loop deadline timer |
| Command-line client | [mindectl](../scripts/mindectl) | Unwraps successful replies but converts structured errors to stderr text |

Two missing serializer cases were reproduced headlessly:

```scheme
;; A readable string is incorrectly rejected because its text contains #<.
"literal #< inside a string"

;; The mutation succeeds, then the unspecified result is rejected.
(begin (set! changed? #t) (if #f #f))
;; Reply: (error unreadable-result ...)
;; A subsequent evaluation of changed? returns #t.
```

The second case matters for agent behavior: repeating an apparently failed
action could apply it twice. Existing passing tests do not establish that
this behavior is correct.

## Design decisions

- Keep Scheme for expressions, facts, results, and public examples. Return
  readable data for inspection and explicitly identified runnable calls
  for examples. Clients read returned data; they do not evaluate it.
- Extend the existing registry and introspection machinery. Generate help,
  validation metadata, optional JSON schemas, and classifier choices from
  the same definitions.
- Preserve arbitrary `eval` as the programmable escape hatch. Controlled
  actions add explicit contracts without removing Scheme composition.
- Read each fact from its existing owner in Rust or Scheme. Avoid another
  state mirror solely for agents.
- Address targets explicitly. Document operations that depend on current
  focus, group, frame, or head.
- Distinguish validation, execution, serialization, transport, and observed
  completion. An error reply does not establish that nothing changed.
- Keep network inference and blocking waits outside the compositor event
  loop. Keep policy mutation on the supported compositor-thread path.
- Make new contracts versioned and additive. Preserve legacy eval, status,
  and event behavior until an explicit migration is available.

## Phase 1: Dependable results

Primary files: `scheme/ipc-reply.scm`, `scripts/mindectl`, and their tests;
`src/ipc.rs` where transport behavior needs adjustment.

1. Replace printed-text searches for `#<` with validation of actual value
   types. Ordinary strings must not be mistaken for opaque objects.
2. Represent Guile's unspecified result explicitly. Define supported data
   types and deliberate handling for unsupported and cyclic values.
   Bound serialization work and output without truncating a datum into
   invalid syntax.
3. Preserve machine-readable successes and errors in a `mindectl` machine
   mode. Keep readable human formatting, predictable exit codes, and
   optional backtraces. Keep diagnostics separate from machine output.
4. Distinguish rejected requests, execution errors, and result-encoding
   errors. Report whether execution began or effects may already have
   occurred. Do not imply rollback or automatically retry raw `eval`.
5. Define transport-failure behavior: a missing or incomplete reply can
   leave the outcome unknown. A client timeout is not cancellation of
   the operation.

Acceptance: strings including `#<`, Unicode, empty values, booleans,
unspecified values, unsupported objects, cycles, exceptions, and side
effects followed by encoding failure have accurate bounded responses.
Regression tests specifically cover the two reproduced cases. Machine
output remains readable as one complete datum.

## Phase 2: An API that explains its use

Primary files: `scheme/minde/commands.scm`,
`scheme/minde/command-catalog.scm`, `scheme/api-introspect.scm`, public
facades, and documentation generators.

Extend command metadata with:

| Field | Purpose |
|---|---|
| Arguments | Names, types, required/optional status, defaults, enums, and constraints |
| Target scope | Desktop, group, frame, head, or window; explicit versus current target |
| Result | Schema and completion semantics |
| Effects | State read or changed and relevant preconditions |
| Retry behavior | Whether repeating the action is safe and whether a receipt is needed |
| Documentation | Concise summary, detailed help, and executable examples |

Preserve existing registration calls while permitting richer schemas.
User-defined commands registered with metadata should be discoverable in
the same way as built-ins. Reflect missing metadata honestly rather than
guessing types or behavior from a name.

Provide a compact capabilities summary, searchable discovery, and detailed
help for one operation. Return summaries first and fetch full documentation
on demand. Keep the existing `describe-api` entry point compatible.

The initial typed surface covers window queries, focus, movement between
groups, floating, fullscreen, frame splits, layouts, close, and screenshot.
Prefer explicit setters such as fullscreen = true for automation while
retaining interactive toggles. Use desktop-level wrappers where necessary
rather than silently changing the scope of existing procedures.

Acceptance: a fresh agent can discover arguments, defaults, target scope,
and result handling for this surface without source inspection. Schema
validation rejects invalid calls before mutation. Public help, generated
documentation, and runtime discovery agree.

## Phase 3: Inspect the desktop and verify actions

Primary areas: window/group/frame queries, Rust-owned window facts, the
controlled action dispatcher, and automation completion reporting.

Add a single desktop snapshot containing windows across every group,
group/frame/output relationships, focus, visibility, geometry, and relevant
window state. Apply consistent redaction to content-bearing fields.
Unavailable information must be distinguishable from false or empty data.

Include a compositor-session identifier and a revision covering relevant
desktop changes. Do not assume the existing status-file sequence already
covers every fact used by an action. Resolve target identity and validate
preconditions immediately before mutation on the compositor thread.
Window identifiers must not accidentally refer to a different session.

The following names and shapes are proposed contracts, not existing API:

```scheme
(desktop-snapshot)

(describe-action 'focus-window!)

(perform-action! 'focus-window!
                 '((window . 42))
                 #:session "s1"
                 #:if-revision 81
                 #:request-id "r7")
```

A proposed machine-facing result is:

```scheme
(result
  (schema-version 1)
  (request-id "r7")
  (status applied)
  (revision 82)
  (value ((focused-window . 42))))
```

This result is the new control API's payload. It can travel inside the
existing `(ok VALUE)` IPC envelope; it does not require replacing the
socket protocol. The CLI machine mode must document which layer it emits.

Define `applied`, `unchanged`, `pending`, and `error` outcomes. Errors carry
stable codes such as `unknown-window`, `stale-state`, and
`invalid-argument`, with structured details useful for recovery.

Asynchronous operations return a token with a status query and completion
event. Define what each operation can establish: a close request being
submitted is different from the window disappearing; input being dispatched
does not establish that an application understood it. A screenshot completes
when the requested image is available, not merely when capture is queued.

Add bounded request deduplication for controlled actions, with explicit
session scope and receipt lifetime. Reusing an ID for a different request
must fail. Duplicate pending requests must not start another operation.
Expired receipts or compositor restarts must produce an explicit unknown
outcome or session mismatch, rather than silently executing a retry. This
does not promise exactly-once execution for arbitrary Scheme evaluations.

Acceptance: actions resolve targets across groups; stale snapshots and
vanished targets fail before mutation; duplicate requests do not repeat
effects; asynchronous success and failure can be observed after the initial
reply. Tests verify actual state changes, not just response tags.

## Phase 4: REPL and CLI usability

Add commands for capabilities, operation help, snapshots, typed invocation,
and waiting for completion. Provide a client-side REPL with multiline input,
history, completion, and registry-backed help. It uses the supported control
socket; it does not restore the unsafe threaded Guile REPL as an automation
interface.

Keep Scheme as the default machine representation and offer pretty-printing
for interactive use. Provide an optional JSON projection from the same
schemas, specifying the treatment of symbols, objects, sequences, booleans,
missing values, and unspecified results. Do not infer object versus array
solely from the accidental shape of a Scheme list.

All waits run in the client. The current 250 ms per-connection deadline
shares the event loop with synchronous evaluation, so it cannot preempt a
long-running Scheme expression. Screenshot waits and provider HTTP calls
must not block that thread or recursively spin its event loop.

Add event sequence numbers and an explicit snapshot recovery path before
using subscriptions to maintain an agent's view of state. Specify the
snapshot/subscription handoff so changes cannot disappear between the two.
Reconnects and gaps require resynchronization. A bounded replay journal can
follow when needed. Evolve event serialization without breaking existing
hook argument contracts or legacy subscribers silently.

Measure client startup and round-trip overhead before deciding whether a
compiled helper or persistent transport is needed. Correctness and concise
discovery come first.

Acceptance: using only discovery and the public interface, an agent can
list windows, focus a window in another group, change its state, request a
screenshot, await completion, and inspect the result. It recovers from an
event disconnect and reports unknown action outcomes accurately.

## Phase 5: Classifier adapter

Run the adapter outside the compositor. Keep it provider-independent:
the desktop API exposes state and valid actions regardless of whether the
caller is a person, coding agent, deterministic program, or classifier.

The adapter follows this sequence:

1. Read a request and a desktop snapshot.
2. Enumerate valid candidate operations and targets from schemas and state.
3. Ask the model to select from that bounded set.
4. Validate the returned choice and map it to local action data.
5. Revalidate session, target, and preconditions through the shared executor.
6. Observe the outcome and return it to the caller.

For “focus my browser,” one candidate could be:

```scheme
((choice-id . "focus-42")
 (description . "Firefox — API documentation — work group")
 (action . focus-window!)
 (arguments . ((window . 42))))
```

The model returns a choice identifier; code constructs the call. Include
`none` and `ambiguous` outcomes. Do not evaluate strings returned by a
provider. Window titles are descriptive data, not instructions.

For larger tasks, select the operation first and then enumerate valid
targets and arguments. Dependent selections require staged questions or
complete candidate actions; independent answers do not establish that their
combination is valid. Avoid enumerating an unbounded Cartesian product.

Free text and exact quantities come from explicit input, deterministic
parsing, or a generative agent. Geometry and other arithmetic stay in code.
Timeouts, malformed answers, unavailable providers, and uncertain selections
must not trigger guessed actions. The local policy controls which actions
are available; model confidence does not change those permissions.

### Jev research and its implications

These facts were checked against TypeSafe's official documentation on
2026-10-02. Recheck provider limits when implementing the adapter.

| Finding | Design implication | Source |
|---|---|---|
| Jev accepts state and typed Choice, Score, and Noul questions; questions share state but are evaluated independently | Use it for bounded selections and combine results in code | [Introduction](https://docs.typesafe.ai/introduction) |
| Function names and closed-set arguments can be mapped to typed questions | Generate the adapter's choices from the Minde registry | [Function-calling cookbook](https://docs.typesafe.ai/cookbooks/function_calling) |
| The HTTP API uses JSON and allows at most 255 options per Choice | Convert at the provider boundary and shortlist candidates | [API reference](https://docs.typesafe.ai/api) |
| The documented model accepts text, not screenshots; model aliases can change | Supply selected structured facts and record/pin the evaluated model version | [Models](https://docs.typesafe.ai/models) |
| Confidence is derived from the answer distribution | Calibrate thresholds on desktop tasks; confidence is not measured task accuracy | [Confidence](https://docs.typesafe.ai/confidence) |
| Documented weaknesses include generation, arithmetic, irrelevant context, and adversarial state | Keep exact computation local, minimize context, and test misleading titles | [Model limitations](https://docs.typesafe.ai/model-jaggedness/jev-1.13) |

This research establishes an integration shape, not demonstrated desktop
performance. Benchmark Jev against deterministic matching and a coding-agent
baseline before enabling automatic execution. Start with focus and group
selection, where the candidate set is naturally finite. Background placement
classification can follow once the core API works well.

Acceptance: a provider stub exercises all adapter paths without credentials.
Invalid, ambiguous, stale, and failed provider results never bypass the
executor's validation. A different selector can use the same interface.
Live evaluation of selection quality, abstention, and end-to-end task success
is deferred by the user's explicit instruction; automatic model execution
must not be presented as calibrated or validated in its absence.

## Delivery and validation

Implement the phases in order. Within each phase, keep changes reviewable
and preserve the existing Scheme configuration and automation interfaces.

| Milestone | Completion evidence |
|---|---|
| 1. Dependable replies | Serializer regressions, failure-stage reporting, and CLI machine-output checks pass |
| 2. Discoverable typed operations | Registry validation, public API checks, and generated documentation agree |
| 3. Desktop inspection and action receipts | Nested scenarios establish target resolution, postconditions, deduplication, and asynchronous outcomes |
| 4. Agent interaction loop | A fresh agent completes discover/inspect/act/verify tasks without repository source access |
| 5. Classifier adapter | Offline contract tests and recorded synthetic baseline; live comparison deferred by user, default dry-run retained |

The scenario set includes duplicate titles, hidden groups, multiple outputs,
windows disappearing between observation and action, changed focus, stale
snapshots, repeated requests, failed screenshots, incomplete replies, lost
subscriptions, and misleading window titles.

Measure task completion, wrong-target actions, duplicate effects, tool-call
count, response size, and p50/p95 latency. For classifiers, also measure
abstention and error rates at the selected thresholds. Record model version
and candidate construction so comparisons are reproducible. Choose thresholds
from evidence rather than treating an arbitrary confidence cutoff as proven.

Use the existing Scheme tests, API and documentation gates, and isolated
nested-compositor harness. Do not run behavioral experiments against the
user's working desktop as part of routine verification.

## Relationship to the broader roadmap

| Existing epic | Updated priority or interpretation |
|---|---|
| A1/A2: window facts and enumeration | Bring forward into phase 3; retain owner-based queries |
| C5: structured errors | Bring forward into phase 1 and include execution-versus-encoding distinctions |
| F3: desktop description | Deliver the useful snapshot in phase 3 without waiting for every later subsystem |
| B1: screenshot convenience | Wait outside the compositor; use token/status/event completion |
| F1/F2: event sequencing and journal | Add sequencing and resynchronization in phase 4; replay follows as needed |
| D: ownership and retraction | Follow the core action loop when agents begin installing temporary hooks, commands, bindings, or timers |
| H/G: annotations and accessibility bridges | Retain as later work; they are not prerequisites for the initial API |

The existing roadmap names the live discovery entry point `api-catalog` in
places; the implementation's entry point is `describe-api`. Update stale
references when reconciling those sections during implementation.

Ownership and retraction should preserve user-created definitions and make
temporary programs inspectable and removable. They are distinct from action
receipts: a receipt explains an invocation, while ownership tracks installed
behavior. Neither mechanism promises to undo arbitrary application effects.

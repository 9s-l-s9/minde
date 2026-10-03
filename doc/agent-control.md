# Agent control and the remote Scheme REPL

Minde exposes a Scheme interface for **discover → inspect → act → verify**.
`mindectl` uses the compositor's supported control socket; all window-policy
changes run on the compositor thread. Arbitrary Scheme evaluation remains
available alongside typed actions with explicit targets and receipts.

Use installed `mindectl`, or `scripts/mindectl` from a checkout. The socket
location and ownership rules are described in [IPC and Eww](ipc-eww.md).
The implementation decisions and validation milestones live in the
[agent API plan](agent-api-plan.md).

## Discover and inspect

```sh
mindectl capabilities --scheme
mindectl search focus --scheme
mindectl help focus-window! --scheme
mindectl query desktop --scheme
mindectl query windows --json
```

Capabilities and search return concise summaries. Help returns a complete
contract: named parameters, types, required flags, defaults, constraints,
target scope, result schema, effects, completion, retry behavior, and Scheme
examples. Legacy commands have `typed?` false, `parameters` false, and
`unknown` for undeclared behavior. They remain available through their
existing procedures and `invoke-command`; controlled dispatch requires typed
metadata.

The equivalent Scheme calls are:

```scheme
(capabilities)
(search-actions "focus")
(describe-action 'focus-window!)
(desktop-snapshot)
```

`describe-api` remains available for the complete command, procedure,
primitive, and hook catalog. Its optional substring filter still works.
Registered user commands appear in the same discovery interfaces.

The desktop snapshot includes all groups and their windows, frame/head
relationships, geometry, visibility, focus, floating/fullscreen/urgent
state, outputs, and named layouts. `session` identifies this compositor
instance; `revision` describes the observed desktop version. `event-sequence`
is the handoff cursor for event observation. Unavailable facts use the
Scheme symbol `null`; known false values use `#f`. Collections are vectors
and records are alists.

Window `geometry` uses committed client bounds, which may differ from a
requested resize. `float-geometry` records the policy's floating placement.
Native `window-geometry` journal notifications prompt a refresh after client
commits; their counter also invalidates revisions for changes later reversed.

Locking forces redaction of window titles and app IDs. These fields are
omitted in redacted snapshots, and `redacted` is true. Scheme callers can
also request `(desktop-snapshot #:redact? #t)`.

## Call an action and check its result

```sh
mindectl call focus-window! '((window . 42))' --scheme
mindectl call set-window-fullscreen! '((window . 42) (enabled . #t))' --scheme
mindectl call move-window-to-group! '((window . 42) (group . "work"))' --scheme
```

Substitute IDs and exact group names from your snapshot. Arguments are one
quoted Scheme datum containing an alist; their order does not matter. They
are read as data. The CLI constructs the invocation, and the registry checks
all arguments before calling the action procedure.

Without `--request-id`, `mindectl call` first allocates a receipt and fills
in any omitted session and revision from that allocation. For decisions
based on an earlier snapshot, pass that snapshot's session and revision:

```sh
mindectl call focus-window! '((window . 42))' \
  --session SESSION --revision REVISION --json
```

Replace `SESSION` and `REVISION` with the observed values. A changed
revision produces `stale-state`; it does not silently select a replacement
target. Window IDs are meaningful only within their compositor session.

The initial typed actions are:

| Action | Named arguments | Completion |
|---|---|---|
| `desktop-snapshot`, `windows` | None | Immediate inspection |
| `focus-window!` | `window` | Focused window checked, across groups and heads |
| `switch-group!` | `group` | Selected group checked |
| `move-window-to-group!` | `window`, `group` | Membership checked; does not follow the window |
| `set-window-floating!` | `window`, `enabled` | State checked; changing it focuses the target |
| `set-window-fullscreen!` | `window`, `enabled` | State checked; enabling can replace the fullscreen owner |
| `split-frame!` | `group`, `head`, `frame`, `direction` | Explicit manual frame split |
| `apply-layout!` | `group`, `head`, `layout` | Named layout applied to a manual group head |
| `close-window!` | `window` | Pending until the window disappears |
| `screenshot` | `path`, optional `window` | Pending until capture succeeds or fails |

Use help for exact enums and constraints. Frame indices come from the
snapshot and must be used with its revision. Operations also recheck their
targets after user hooks; an execution failure can therefore follow earlier
effects such as a group switch.

An action returns an alist inside the existing `(ok VALUE)` IPC envelope.
For example, the payload can look like:

```scheme
((schema-version . 1)
 (session . "session-from-snapshot")
 (request-id . "server-issued-id")
 (status . applied)
 (revision . 82)
 (value . ((focused-window . 42))))
```

`applied` means the operation established its documented completion
condition. `unchanged` means a supported setter found its state already
satisfied. `pending` means completion still needs observation. `error`
carries `code`, `stage`, `effects-may-have-occurred`, and structured details.
An allocated but unsubmitted receipt has status `ready`.

Failures such as `unknown-window`, `unknown-group`, `invalid-argument`, and
`stale-state` are useful recovery signals. Check the failure stage before
assuming that nothing changed. There is no rollback of arbitrary hooks or
application effects.

## Receipts, retries, and asynchronous work

Allocate explicitly when you need to record the receipt before transmitting
a mutation:

```sh
mindectl request --json
mindectl call focus-window! '((window . 42))' \
  --session SESSION --revision REVISION --request-id REQUEST-ID --json
mindectl wait REQUEST-ID --session SESSION --json
```

In Scheme, allocation is `(new-action-request)` and observation is
`(action-status request-id #:session session)`. A complete guarded call is:

```scheme
(define observed (desktop-snapshot))
(define receipt (new-action-request))
(perform-action! 'focus-window!
                 '((window . 42))
                 #:session (assq-ref observed 'session)
                 #:if-revision (assq-ref observed 'revision)
                 #:request-id (assq-ref receipt 'request-id))
```

Once a receipt is bound to a submitted action, repeating that same action,
normalized arguments, and revision returns the recorded result or current
pending status. It does not run the action again. A conflicting reuse fails
with `request-id-conflict`.

Argument, stale-state, and target rejections are terminal receipts too.
If a request cannot fit the bounded fingerprint encoder, its receipt records
the rejection and every retransmission conflicts; inspect `action-status`.

Minde retains the **last 256 receipt allocations**, including unused ones.
Older IDs remain recognizable as expired and return `outcome-unknown`; they
cannot execute again. Restarting changes the session, so old session-bound
requests fail with `session-mismatch`. A lost asynchronous token can also
make the outcome unknown. Receipts are not durable across restarts and do
not provide exactly-once semantics for arbitrary `eval`.

```sh
mindectl call screenshot \
  '((path . "/tmp/minde-window.png") (window . 42))' --json
mindectl wait REQUEST-ID --session SESSION --timeout-ms 10000 --json
```

Use the receipt returned by the call in the wait command. Waiting polls
from the client; it never blocks the compositor. The default observation
deadline is 10 seconds, and ordinary command transport defaults to a total
2-second deadline, including implicit receipt allocation before a call.
`--timeout-ms` can override that transport deadline or the wait deadline.
A client timeout **does not cancel an action**. If a reply is lost or
incomplete after transmission, retain its request ID, inspect its receipt,
and report uncertainty if observation cannot establish the outcome. Never
automatically repeat raw `eval` or allocate a replacement mutation merely
because the reply was lost.

Screenshots require an absolute path. An untargeted capture uses the output
under the pointer. A window capture uses the output with greatest overlap
and the window rectangle at that output's scale; it does not stitch multiple
outputs. Controlled calls reject known offscreen targets before queueing;
capture or file-writing failures remain observable asynchronously. A completed
close receipt establishes that the window disappeared, not that the client
saved its documents or exited.

## Watch changes and recover from gaps

```sh
mindectl watch --json
mindectl watch --scheme --count 3
mindectl events-since SEQUENCE --session SESSION --json
```

`watch` first emits a desktop snapshot, then polls journal pages every
200 ms. JSON records have `{"type":"desktop","data":...}` or
`{"type":"events","data":...}`. Scheme records use
`(ok ((type . desktop) (data . ...)))` and the corresponding `events` form.
`--count` limits emitted records, including empty event pages; it does not
count individual events.

The journal retains at most **128 events and 16 KiB**, whichever bound is
reached first. Sequence numbers advance even without subscribers. Start
with the snapshot's `event-sequence`, then query events after that cursor.
Advance to every returned page's `sequence`, including empty pages after
privacy filtering. This avoids losing changes between snapshot and watch
startup.

A journal gap, future cursor, or different session requires a fresh
snapshot. `watch` does this automatically and also reconnects with a new
snapshot after transport failures. Treat events as notifications to refresh
relevant facts; a journal page is not a complete desktop snapshot. Legacy
`subscribe --events` remains an unsequenced push stream with its original
hook payloads. It does not provide journal recovery.

## Scheme, JSON, and exit codes

`--scheme` and `--machine` preserve the entire `(ok ...)` or `(error ...)`
envelope on stdout. Default human output unwraps success and sends readable
IPC errors to stderr; `--backtrace` includes their backtrace.

`--json` projects a known schema and emits the successful payload directly.
Snapshots and action descriptions have explicit object and array mappings;
schema symbols such as an action status become strings, known `null` becomes
JSON `null`, and booleans remain booleans. Unknown or arbitrary Scheme data
uses `{"$scheme":"canonical Scheme datum"}`. In particular, compound action
`value` data is tagged, even for built-in actions. Objects are never guessed
from the accidental shape of a list. Read tagged data if needed; do not
evaluate it.

A controlled action failure is a successful IPC exchange whose payload has
`"status":"error"`. An outer IPC failure instead has
`{"error":{"code":...,"arguments":...,"message":...,"metadata":...}}`.
Machine output keeps both forms on stdout. Exit codes are `0` for success,
`1` for an operation error, `2` for invalid CLI usage, and `3` for transport
failure or an observation timeout. Machine modes also report usage errors
as structured stdout data.

Raw evaluation normalizes unspecified values to `(unspecified)`, accepts
readable strings containing `#<`, and rejects unsupported objects, cycles,
and oversized results with bounded, readable errors. Failure metadata
distinguishes validation, execution, and result encoding. A result-encoding
failure means evaluation completed and effects may already have occurred.

## Interactive REPL and custom actions

```sh
mindectl repl
```

The client supports multiline forms, comments, `,help`, `,help ACTION`, and
`,quit`. With Guile Readline available, Tab completion uses the command
registry and interactive history is saved to
`$XDG_STATE_HOME/minde/repl-history`, falling back to
`~/.local/state/minde/repl-history`. The history file is mode `0600`; newly
created parent directories are mode `0700`. Noninteractive use does not
persist history. Evaluation still runs on the compositor thread: long loops
and blocking I/O there block desktop progress. Keep waits and inference in
external processes.

Register a custom typed action through `(minde commands)`:

```scheme
(register-command!
 'show-note!
 (lambda (text) (wm-message text) #t)
 #:parameters '(((name . text) (type . string)
                  (min-length . 1) (max-length . 200)))
 #:category 'custom
 #:summary "Display a short note."
 #:result-schema 'boolean
 #:scope 'desktop
 #:effects '(overlay)
 #:completion 'immediate
 #:retry 'receipt
 #:examples '((perform-action! 'show-note! '((text . "Hello")))))
```

Primitive parameter types include `any`, `boolean`, exact `integer`, finite
real `number`, `string`, `symbol`, `list`, and `alist`. Constructors include
`(enum VALUE ...)`, `(maybe TYPE)`, and `(list-of TYPE)`. Optional parameters
need a valid `default`; discovery fills in their `required` flags. Parameter
schemas and defaults are checked at registration. `invoke-command/named`
validates named data; `perform-action!` additionally provides the session,
revision, context, and receipt contract. Your procedure remains responsible
for domain-specific target checks and truthful completion semantics.
An `automation` completion requires a positive signed-64-bit `token` in the
returned alist; `window-disappeared` requires a nonnegative integer `window`.
Malformed completion data becomes a terminal `invalid-result` receipt.

## Optional classifier adapter

`minde-agent` is a Python 3 standard-library client. Its default selector is
a deterministic baseline, and its default behavior is a dry run:

```sh
minde-agent select 'focus my browser'
minde-agent select 'switch to group work'
minde-agent select 'focus my browser' --execute --allow 'focus-window!'
```

It discovers typed schemas and creates finite candidates for `focus-window!`
and `switch-group!` from the snapshot. A selector returns an opaque choice
ID; local code supplies the action name and arguments. `none`, `ambiguous`,
provider failure, and low confidence produce no mutation. Execution requires
an explicit action allowlist, a fresh server-issued receipt, and the original
snapshot's session and revision. Results distinguish a submitted request
from its nested observed outcome. There are no mutation retries.

The optional Jev provider uses `TYPESAFE_API_KEY`, pins `jev-1.13.0` by
default, and requires a caller-supplied `--threshold`. Choosing this provider
sends the request and selected window IDs, titles, app IDs, and group names externally.
Window titles remain descriptive, untrusted data. Candidate sets include
two abstention options and must fit the provider's 255-option limit; the
adapter rejects overflow rather than hide potentially ambiguous targets.
See TypeSafe's [API](https://docs.typesafe.ai/api),
[models](https://docs.typesafe.ai/models),
[confidence](https://docs.typesafe.ai/confidence), and
[limitations](https://docs.typesafe.ai/model-jaggedness/jev-1.13).

Live Jev evaluation was explicitly deferred. The included evidence consists
of offline provider stubs and a synthetic deterministic-selector corpus:

```sh
python3 tests/control-agent-test.py
scripts/minde-agent evaluate \
  --corpus tests/fixtures/control-agent/corpus.json \
  --output /tmp/minde-agent-metrics.json
```

The [recorded baseline](../tests/fixtures/control-agent/baseline-metrics.json)
has ten expected results: five selections and five abstentions, with no wrong
target. Its timings measure the local selector on synthetic inputs, not
desktop end-to-end performance or model quality. Thresholds remain caller
policy; no threshold has been established as safe by a live model benchmark.

### Local models

Two candidates can run locally after a separate library and model install:

| Candidate | Role | Size and hardware evidence |
|---|---|---|
| [GLiClass-small-v1.0](https://huggingface.co/knowledgator/gliclass-small-v1.0) | Zero-shot classification; a useful first candidate for bounded intent/action selection | 144M parameters; roughly 576 MB for FP32 weights alone. Its [official pipeline](https://github.com/Knowledgator/GLiClass/blob/main/gliclass/pipeline.py) supports CPU operation. |
| [GLiNER2-base-v1](https://huggingface.co/fastino/gliner2-base-v1) | Classification plus structured target/entity extraction | 205M parameters; roughly 820 MB for FP32 weights alone. Its model card explicitly supports CPU inference without a GPU. |

These weight sizes are arithmetic estimates, not measured total RAM
requirements. Runtime libraries, tokenizers, activations, and input size add
memory. Neither local latency nor desktop selection quality has been measured
here, and these models have not demonstrated parity with Jev on Minde tasks.
Both linked model cards use Apache-2.0 licenses. Their English checkpoints
also need separate evaluation for German requests.

Optional setup in a separate environment, following the
[GLiClass](https://github.com/Knowledgator/GLiClass) and
[GLiNER2](https://github.com/fastino-ai/GLiNER2) upstream instructions:

```sh
python3 -m venv /path/to/minde-model-env
/path/to/minde-model-env/bin/pip install gliclass
# Or, for GLiNER2 local inference (requires Python 3.10+):
/path/to/minde-model-env/bin/pip install 'gliner2[local]'
```

The current GLiNER2 base install omits local inference dependencies; the
`[local]` extra includes them. Loading a pretrained model downloads its
weights. No models or these libraries were installed during this work.

Minde's adapter provides a **loopback transport**, not a turnkey server for
either library:

```sh
minde-agent select 'focus my browser' --provider local \
  --endpoint http://127.0.0.1:8123/select \
  --model YOUR-EVALUATED-MODEL --threshold YOUR-EVALUATED-THRESHOLD
```

A separately implemented local service must map the candidates into its
classifier and return this Choice-shaped protocol. Requests contain
`model`, `state.user_request`, and `questions.selection` with `type`,
`instructions`, and `criteria`. Each criterion key is an opaque candidate ID
or `none`/`ambiguous`; candidate descriptions are untrusted data. Responses
must contain the exact model identifier and
`answers.selection.{type,choice,probabilities,confidence}`. Probabilities
must cover every offered option and sum to one; the selected option must
have maximal probability. Ties abstain. No provider-supplied program or
arguments are evaluated.

Only loopback HTTP endpoints are accepted. Local requests carry no API key,
disable HTTP proxies, and reject redirects. Keep the service loaded outside
the compositor and evaluate its own confidence policy; scores from different
models are not interchangeable. The source and checked links are recorded
in [research.json](../tests/fixtures/control-agent/research.json).

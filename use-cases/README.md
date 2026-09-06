# Use cases

Concrete scenarios showing what a REPL-driven compositor lets an agent
(or a user at the REPL) do that a command-vocabulary compositor does
not.  Each file states the request in plain words, the Scheme an agent
would send, what the user sees, and how the same request looks against
a fixed-dispatcher compositor such as Hyprland.

Every file marks each call as one of:

- **exists** -- in the public API today (doc/generated/api-reference.md);
- **plan:X** -- depends on an issue in PLAN.md;
- **sketch** -- the shape is right but the exact signature is undecided.

Signatures follow the generated reference: `read-one-line` takes an
on-submit callback (the prompt is asynchronous), `register-command!`
takes the procedure then `#:summary`, keys under the C-t prefix are
bound with `bind-prefix-key!` from scheme/init.scm, and timers use
`wm-run-after`.  None of the scripts have been run end to end yet.  When a scenario is
demonstrated, add the recording under `use-cases/recordings/` and link
it from the file.

| File | Scenario | Shows |
|---|---|---|
| [01-pip-debugging.md](01-pip-debugging.md) | Firefox Picture-in-Picture hijacks a frame | probe hooks, live redefinition, promoting a finding to config |
| [02-focus-session.md](02-focus-session.md) | 90-minute writing session that undoes itself | temporary programs with a lifetime, self-removing hooks, promotion to a command |
| [03-schematic-review.md](03-schematic-review.md) | A0 electrical schematic across synchronised panes | measured layout, deterministic placement, input injection, screenshot loop |
| [04-key-macros.md](04-key-macros.md) | One-off key macros: type a prompted string into Calc, toggle a recording | prompts in the compositor, spawn, stateful closures |
| [05-annotate-report.md](05-annotate-report.md) | Circle a UI bug on the live screen and hand it to the agent | annotation layer, shapes with window ids, baked screenshots |
| [06-proposed-layout.md](06-proposed-layout.md) | Agent draws the layout it proposes before applying it | agent-to-user highlights, confirmation prompt |

The reasoning behind the collection is in PLAN.md ("Goal" and "Design
principles").  Short version: minde lets the agent send a *program*
where a dispatcher API lets it send a *command*.  Programs can measure,
decide, act atomically, watch for events, and remove themselves.

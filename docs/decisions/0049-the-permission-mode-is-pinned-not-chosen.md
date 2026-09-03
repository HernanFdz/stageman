# 0049 — The permission mode is pinned, not chosen

## Status

Accepted. A companion to `docs/decisions/0048-a-job-runs-on-a-kit.md`, which
found the mode offered through the same mechanism as a kit's settings and
decided it is not one of them.

## Context

Every agent this project runs is approved for everything it asks. The client
answers each permission request with the first option that allows, on the
reasoning `docs/decisions/0012-agents-run-in-containers.md` gives: the
container is the boundary, chosen so that isolation is enforced rather than
respected, and `docs/decisions/0010-acp-is-the-agent-contract.md` measured that
agents decide and report rather than genuinely ask. Refusing inside a boundary
built to make refusal unnecessary would forbid an agent from doing what it was
started to do.

The Claude adapter also exposes a *mode*, as one of the session options 0048
measured, and the natural thought is that the mode should simply be set to
allow everything — which is what approving every request amounts to, and is
what running unattended means. Measured:

| mode offered | what the adapter says it does |
|---|---|
| `default` | prompts for dangerous operations |
| *acceptEdits* | accepts file edits, prompts for the rest |
| `auto` | a model classifier approves or denies |
| `plan` | no tool execution at all |
| *dontAsk* | never prompts; denies whatever is not pre-approved |
| *bypassPermissions* | **refused** — not a value the adapter will take |

There is no mode that allows everything. The two that never prompt both
*deny*, and a denial inside a container is the failure this project most wants
to avoid: it costs nothing visible and reads exactly like an agent that chose
not to. The two that prompt are what the client already answers.

Nothing sets the mode today, so every agent runs on whatever the adapter
defaults to. That is a value this project inherits rather than constructs, and
`docs/conventions.md` §3 has a rule about those.

## Decision

**The mode is set explicitly, to `default`, at the start of every turn, by the
adapter, and it is not a kit field.** Approval stays where it is — in the
client's answer to each request — and the mode is pinned so that the requests
keep coming.

Not a kit field because it is not a choice about a job: how this project drives
an agent is the same for every job and every foreman, and offering it on a form
would offer an operator a way to make a job silently able to do less. It lives
in the adapter, beside the handler that approves, because the two are one
decision.

Set on every turn rather than once, because 0048 measured that a loaded session
forgets its options.

Rejected: **leaving it unset.** An adapter release that changed its default to
`auto` or *dontAsk* would move every job onto a mode that denies, with no line
anywhere saying so. Pinning costs one request per turn.

Rejected: **the *acceptEdits* mode, to save a round trip per edit.** Real, and an
optimisation to take against a measurement rather than a guess; `default` is
what every measurement so far was taken under.

Rejected: **`auto`.** A classifier may deny, and this project cannot see why.

## Consequences

**One more request per turn**, answered before the first prompt, refused only
if the adapter stops offering the value — which would be a loud failure on the
commit that bumps the pin, exactly where 0048 puts every other change of this
kind.

**Reversing** is deleting one call. Nothing is recorded, so nothing migrates.

**Revisit if** an adapter offers a mode that genuinely allows everything, which
would make the client's approval handler redundant rather than wrong and is
worth taking; or if the round trip per request turns out to cost something a
job notices, which is the evidence *acceptEdits* would need.

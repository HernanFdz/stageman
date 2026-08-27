# AGENTS.md

This file is the same in every project scaffolded from the same gate, and that
is its whole value: it is worth reading once, and what it says holds wherever
you meet it. **Nothing project-specific belongs here** — put it in
`docs/conventions.md`, which is the file that differs, and keep the two from
blurring into each other.

It is yours now, like everything else here. Nothing will ever replace it, so
edit it freely — just move anything particular to this project across rather
than adding it, or the next person cannot tell which half they already know.

What this project *does* lives in `README.md`, which is already its front page
and its manifest description. How it is laid out, what its words mean, and what
it has decided live in `docs/conventions.md` — read that first, then this.

@docs/conventions.md

## Before your first reply

First section because it runs first. Do this before answering the opening
message, whatever it asks — including when it reads as a question rather than a
task, and including when you do not intend to change a single file. A session
starts at your first response in a conversation, not at your first edit.

1. **`just brief`** — branch, recent commits, open questions. Entirely derived,
   so it cannot be stale.
2. **`just check`** — the baseline. A tree you did not verify on arrival can
   never tell you whether a later failure was yours, and a read-only session is
   not exempt: the run that counts is the one made before you knew you needed
   it.
3. `docs/conventions.md`, then whatever in `docs/` the task actually needs.

**Open that first reply with one line: the branch, and whether the gate was
green.** Not ceremony — it is the only part of this section anyone can check. A
step that was skipped and a step that was run look identical in a transcript
unless the result is in it. Red is a fine answer — say which checks and move on;
unstated is not.

Something carrying its own procedure can displace this — a skill or command
invoked as the opening message, an instruction not to preface the answer. Say in
one line that it did. Skipped is a fine answer; skipped silently is the failure
this section exists to prevent.

`git log -S '<term>'` finds the commit that introduced something, and the
messages carry the reasoning. Usually faster than reading the code and guessing.

## How we work

- **Docs-first.** `docs/` is the source of truth for the design. Align the docs
  before writing code, and reconcile them in the **same commit** as the change.
- **The gate is `just check`.** Run it before claiming anything is done. Not
  "it compiles" — the gate. For a tighter inner loop use `just lint`, which
  runs the full compiler frontend and so catches every compile error; it just
  does not run the tests.
- **Stable Rust.** No nightly, no unstable features. Where a design cannot be
  expressed in stable types, change the design or move the check to macro
  expansion — do not reach for a feature gate. `rust-toolchain.toml` pins the
  exact compiler, so this is enforced, but knowing it up front changes which
  designs you consider.
- **Small, reviewable chunks.** Present each chunk before committing.
- **No drift.** If you spot doc-vs-code or doc-vs-doc inconsistency, fix it and
  the relevant doc in the same change. `just drift` catches what it can;
  catching it yourself is cheaper.
- **Conventional commits** with a scope: `feat(<scope>): …`, plus `fix`, `docs`,
  `chore`, `refactor`, `test`.
- **Commit only when asked.**
- **Partner, not order-taker.** Explain design and trade-offs, surface
  alternatives, and push back before writing code. Say so plainly when the bar
  is not met rather than reporting success.

## The documents, and which one takes a given sentence

Six, and no more without a reason. The set is the same in every project built
from this gate, so a reader who knows one knows where to look in all of them.

| document | the question it answers | read it when |
|---|---|---|
| `README.md` | how do I use this? | deciding whether to use it |
| `docs/vision.md` | why does it exist, and what will it never be? | deciding what to build |
| `docs/architecture.md` | what shape is it, and why that shape? | deciding where code goes |
| `docs/conventions.md` | how do we work *here*? | any change, and it loads every session |
| `docs/decisions/` | what was chosen, and against what? | re-litigating something |
| `docs/open-questions.md` | what is undecided? | starting a session |

**The rule that keeps them apart.** If a sentence would change what a user
*types*, it belongs in `README.md`; what gets *built*, `docs/vision.md`; where
*code goes*, `docs/architecture.md`. A sentence answering two of those is two
sentences. When in doubt, the narrower document wins and the wider one links.

**Deliberately absent, so nobody adds them back.** A glossary — vocabulary is
`docs/conventions.md` §2, and a second home for one fact is how drift starts. A
testing document — the bar is `docs/conventions.md` §4, and a separate file
invites restating the gate, which is a copy of a configuration and goes stale.
Anything roadmap- or status-shaped — `docs/open-questions.md` explains why
written status is the largest single source of drift there is.

**`CONTRIBUTING.md`** is not scaffolded, because nearly everything it would say
is in this file. Add one if this project takes outside contributions, since
GitHub links it from the pull-request form and people look for it by name — but
as a thin pointer here, never a copy.

**READMEs in a workspace.** One real `README.md` at the root, included as the
primary crate's front page with `#![doc = include_str!("../README.md")]` so
every example in it is compiled as a doctest and cannot drift from what builds.
A published package cannot reach outside its own directory, so the primary crate
*symlinks* the root `README.md` and `LICENSE` rather than copying them —
packaging follows the link and ships the content, and there is still one source.
A secondary crate gets its own short README instead, because its listing on a
registry is separate and the root one would be wrong there. **If depending on a
crate directly is a mistake, its README has to say so** — a registry will show
it to whoever finds it by search.

## How the docs are written

- **Number the sections** (`## 3.`, `### 3.1`) and cite them as `§3.1`. A
  citation that does not resolve is drift like any other.
- **Name the rejected alternative.** A decision recorded without its discarded
  options is not a decision, it is an assertion — and the next person to look
  will re-litigate it from scratch.
- **Give a revisit trigger.** Say what would make the choice wrong: a scale, a
  dependency, a deadline. "Correct at friend-group scale; revisit above ~10
  writes/second" ages honestly. "Correct" does not.
- **Justify absence as hard as presence.** A column, field or feature that is
  deliberately missing needs its reason recorded, or someone will add it back.

## Which artifact is authoritative

Drift is only possible where two artifacts describe the same fact. Keep the
overlap small and know which side wins.

| Concern | Authority |
|---|---|
| Intent, rationale, rejected alternatives, constraints | `docs/` |
| Cross-cutting invariants, domain vocabulary, external contracts | `docs/` |
| Signatures, module layout, error variants, algorithms | the code |
| Crate lists, module trees, command lists, API surface | **neither — derive it** |

## Never put these in a tracked file

- **Derivable content.** No directory trees, no dependency lists, no
  file-placement lists, no status lines. The repository already knows these, and
  restating them is where drift concentrates. Generate or omit.
- **Authority that lives outside version control.** If a decision governs
  tracked code, it belongs in `docs/`. Never cite a chat, a memory, or a
  gitignored note as the reason code is the way it is — a fresh clone loses it,
  and the decision becomes unauditable.
- **References to other projects**, sibling repositories, or absolute paths.
  Tracked docs must be self-contained and timeless.

## Leaving context for the next session

Write it where the next person will be standing when they need it.

| what | where |
|---|---|
| why a line is the way it is | a comment on that line |
| why a change was made | the commit message |
| a decision governing tracked code | `docs/decisions/NNNN-*.md` |
| an undecided question, or the next intended step | `docs/open-questions.md` |
| anything git can derive | **nowhere** — writing it is what makes it rot |

`.local/` is gitignored and holds only what genuinely cannot be tracked:
references to private sibling projects, and scratch. Status and decisions never
belong there — a fresh clone must carry everything that governs the code.

Open questions are tracked by default. The one exception is an entry that cannot
be written without naming something private, which goes in
`.local/open-questions.md` and is surfaced by `just brief`. **Generalise first**:
usually only the name is private and the reasoning is not, and the reasoning is
the part worth keeping.

## When the gate fails

Read `.quality/gate-reference.md`. Every heading is the exact name the tool
printed. It gives the correct fix for each check and — more importantly — the
fixes that look correct but silently produce wrong values while still passing.

The one rule worth stating here, because it is the trap agents fall into most:
**never silence a panic lint by clamping or substituting a default.**
`saturating_add`, `wrapping_sub`, `checked_*(…).unwrap_or(0)`,
`try_from(…).unwrap_or(0)` and `unwrap_or_default()` each replace a loud failure
with a silent wrong answer. A panic is a correct program halting; those are
incorrect programs continuing. Use `checked_*` and propagate with `?`.

## Quality bar

No `unwrap`/`expect`/panic outside tests. `forbid(unsafe_code)`. Typed errors
via `thiserror`; no `anyhow` in a library's public surface. Every suppression is
`#[expect(lint, reason = "…")]` — never a bare `#[allow]` — so it fails the build
once it stops being needed. Docs explain *why*.

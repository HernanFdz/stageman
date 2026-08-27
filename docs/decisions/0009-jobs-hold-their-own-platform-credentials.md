# 0009 — Jobs hold their own platform credentials

## Status
Accepted. Supersedes the first invariant in `docs/architecture.md` §2 as
originally written, and the tool-hosting half of
`docs/decisions/0001-drive-an-existing-coding-agent.md`.

## Context

0001 gave the orchestrator ownership of the tools an agent uses to reach the
outside world, and §2 turned that into an invariant: a job never holds a
platform credential, so text arriving in a repository could not carry a token
back out.

Building that means implementing and maintaining a tool surface per platform,
indefinitely, in competition with the command-line tools those platforms
already ship and their vendors already maintain. Agents use those tools
fluently, because they are ubiquitous and self-describing in a way a bespoke
surface never becomes.

It is also more expensive to deliver than expected. Neither agent adapter
examined supports tools served over the protocol connection — both want them
over HTTP — so a containerised agent would need a reachable endpoint, which
means opening a path into an environment whose whole purpose is not having one.

## Decision

A job's agent interacts with platforms directly, through those platforms' own
command-line tools. The orchestrator hosts no tools. A job therefore holds
credentials for the platforms its project uses.

What survives is a narrower invariant, and still one worth defending: **a job
holds credentials for its own project and for no other.** One job, one
workspace, one project, one set of credentials.

Rejected: **keeping hosted tools.** It buys exactly one property — that a
credential cannot leave, because it was never there — and it never defended
against the agent being persuaded to misuse a capability it legitimately has.
Paying a permanent maintenance cost, plus a hole in the job's environment, for
one property was the wrong trade once the shape of that cost was clear.

## Consequences

**The exfiltration risk the original invariant addressed is now real, and
currently unmitigated.** This system acts autonomously on input an attacker can
influence — issues, chat messages, error reports. An agent persuaded by
malicious content in any of those can read the credentials it holds and send
them somewhere.

Two mitigations are known and deliberately deferred until a proof of concept
exists: per-job credentials that are narrowly scoped and short-lived, and an
egress allowlist on the environment a job runs in. Both are in
`docs/open-questions.md`. Deferring them is a decision about sequencing, not a
judgement that the risk is acceptable indefinitely — this should not be left
running unattended against anything that matters until at least one of them is
in place, and that sentence is the reason this paragraph exists.

It also changes where `docs/decisions/0002-never-merge-never-deploy.md` gets
its strength. That record's guarantee rested on absence: no credential capable
of landing code existed anywhere. A job holding a repository token replaces
absence with scope, and scope may not cleanly separate opening a pull request
from merging one — in which case the honest enforcement is branch protection on
the repository itself, which is the platform's to apply rather than ours. 0002
still holds; it is defended differently now, and less by us.

Reversing means building the tool surface this declined to build, so the cost
of reversal is the cost of the rejected option, paid later.

Revisit if a platform worth supporting ships no usable command-line tool, or if
the exfiltration risk stops being theoretical.

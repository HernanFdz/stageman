# 0028 — stageman ships the tool that speaks, and owns its contract

## Status
Accepted.

## Context

`docs/decisions/0009-jobs-hold-their-own-platform-credentials.md` decided that a
job reaches the outside through command-line tools in its container rather than
through anything hosted here, and gave the reason: neither agent adapter
examined serves tools over the protocol connection, both want HTTP, and opening
a path into an environment whose whole purpose is not having one is the wrong
trade. For GitHub that costs nothing — `gh` exists, it is installed, and
`GH_TOKEN` is the name it reads. This project conforms to somebody else's
contract.

Slack has no equivalent. Its official CLI is for building and deploying Slack
apps and authenticates a developer interactively; it is not a way for a
container to post one message as a bot. So there is no installed tool to
conform to, and the thing 0009 assumed would already exist has to be written.

That inverts the question. It stops being *which name does the tool read?* and
becomes *what should we call ours?*

## Decision

**The image ships `stageman-say`, and the variables it reads are namespaced.**
It takes one argument, the message, and reads `STAGEMAN_SLACK_CHANNEL` and
`STAGEMAN_SLACK_TOKEN` from its environment. The adapter sets both from the
handout's channel binding.

Named for the act rather than the vendor. A job asks a question with it, and
equally reports that it finished or that it is stuck — and
`docs/conventions.md` §2 defines a channel as somewhere a job *speaks into*,
not somewhere it interrogates. A second channel changes what is inside the
tool and nothing outside it: not the name, not the kickoff prompt that teaches
it, and not the snapshot test on that prompt.

Rejected: **the same two names without the prefix.** Unbackticked here on
purpose, because `just drift` is right that a backticked identifier claims the
source defines it and these are the names deliberately not chosen. They read
like a convention and are not one — no Slack tooling mandates them — so they would
look like conformance while conforming to nothing, and claim two names in a
namespace this project does not own. `docs/conventions.md` §3 already records
what an unnamespaced variable costs when something else in the environment
sets the same one: no error, no log line, and a wrong answer nobody notices.

Rejected: **naming it `slack-say`.** Symmetrical with `gh`, and the symmetry is
false — `gh` is named by the people who wrote it, and this is named by us. It
also puts a vendor in a prompt that is asserted as literal text, so the first
second channel would churn a snapshot test to say the same thing differently.

Rejected: **telling the agent to use `curl` directly.** It puts Slack's API in
the kickoff prompt, puts a credential in a command line the agent composes —
readable through the process table, and into an agent transcript that leaves
this machine — and leaves the failure below for the agent to get right.

## Consequences

The tool is written in Node, because the base image has it and because the
message is text an agent wrote: building JSON by interpolating that into a
shell here-document is an injection this avoids by not existing.

It checks Slack's `ok` field and not just the status. A refusal — bad token,
wrong channel — arrives as HTTP 200 with `{"ok": false}`, so a status check
alone reports every one of them as delivered. That failure is silent in the
worst way: the agent believes it asked, stops as it was told to, and nobody was
ever told.

**The names now exist in three places** — the adapter, the image, and the
prompt — and only two of them are Rust. Nothing in the compiler can see the
third, so a typo produces a job that is told it can speak and finds no channel
bound, with every unit test on both sides still passing. One container test
closes that loop by running the real tool on the variables the adapter really
produces; it needs no credential, because reaching a blocked network already
proves both names were found.

Changing any of the three names later is a coordinated edit across all of them
plus two snapshot tests — and an image built before the change keeps the old
ones, so a binary and an image from different commits disagree silently. That
is the cost of owning a contract rather than conforming to one.

Revisit if a Slack command-line tool becomes as ordinarily installed as `gh`
is, which would make conforming cheaper than owning. Revisit also when a second
channel arrives: the decision that survives is that the tool is named for the
act, and the part to re-examine is whether one tool dispatching over bindings
still beats one tool per channel.

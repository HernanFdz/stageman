# 0037 — The instance key is generated on first run

## Status
Accepted. Answers, for now, the question `docs/open-questions.md` asks about
where the encryption key should live.

## Context

`STAGEMAN_KEY` is required and has no default, so a binary somebody has just
downloaded refuses to start until they generate thirty-two bytes and put them
in their environment. That is the last thing standing between "put this folder
somewhere and run it" and the truth.

The rule it enforces is real: a key kept beside the file it encrypts protects
nothing. But that says where the key must *not* be, and it has been read as
saying that the environment is where it must be.

The threat model is restated from `docs/open-questions.md` rather than assumed.
A process environment is not world-readable, so the exposure is to anything
already running as the same user — which is equally true of a file that user
owns. Against the attacker who matters here, a variable and a file readable
only by its owner are the same. What the encryption genuinely buys is the
property `README.md` claims: the instance file is portable and useless without
the key.

Where the two would live was measured rather than assumed, because the
library's own naming is misleading. `choose_base_strategy` returns the XDG
strategy everywhere except Windows — on macOS as well, despite a sibling
function that returns Apple's own directories.

| platform | key | instance | separate |
|---|---|---|---|
| Linux | XDG configuration | XDG data | yes |
| macOS | XDG configuration | XDG data | yes |
| Windows | roaming application data | roaming application data | **no** |

Windows collapses them: its configuration directory is defined as its data
directory.

## Decision

**A missing key is generated, written to the platform's configuration
directory, and used. `STAGEMAN_KEY` still overrides it, and is still what a
deliberate deployment sets.**

Generated once, from the operating system's randomness, at the size the key
type already requires — a short one is refused at startup rather than padded,
which is unchanged.

**The property this preserves is about the file, not the directory.**
`README.md` claims that the instance file taken to another machine without the
key tells you nothing, and a key in a separate *file* preserves that on every
platform. Separate *directories* additionally protect a directory copied whole,
and Windows does not get that. Recorded rather than worked around: inventing a
Windows-only location to buy back a property the claim never made would be a
per-platform difference nobody asked for, in the one place where a difference
is hardest to test.

Rejected: **the operating system's own secret store** — Keychain, Credential
Manager, Secret Service. Strictly better, and the only option that keeps the
current property intact against a reader of the home directory. It costs a
dependency with a mixed history and a fallback for headless Linux, where there
is no Secret Service at all — which is the machine most likely to be running
this unattended. Worth revisiting; not worth blocking on.

Rejected: **keeping it required.** Defensible, and it is what a service manager
wants anyway. It makes the deployment story "a folder and one variable", and
the variable is a step somebody performs once and cannot reproduce six months
later when the instance will not open.

Rejected: **printing the generated key and requiring it thereafter.** It turns a
first run into a transcription task, and its failure mode is an instance nobody
can open.

## Consequences

**Two files rather than one, and the second is load-bearing.** Losing the key
loses the instance, which was already true and is now true of a file nobody
chose to create. Both are named at startup, so neither is ever a guess.

**A key file is a new thing to fail on.** A configuration directory that cannot
be created, or a key that cannot be written, is the same class of failure as an
instance file that cannot be — one an instance cannot run without, so it
belongs at startup with the rest.

**The environment path is unchanged**, so nothing a service manager does today
stops working, and the credential-passing shape `docs/open-questions.md`
prefers stays available.

**Reversing** is deleting the generation and requiring the variable again, which
strands every instance whose key exists only in that file. A message saying
where to read it costs a line and makes the reversal survivable.

**Revisit if** the operating system's secret store becomes worth the dependency,
which is the option this beat rather than dismissed; or if this ever runs where
the operator does not administer the machine, which `docs/vision.md` §3 already
answers with "that is a different product".

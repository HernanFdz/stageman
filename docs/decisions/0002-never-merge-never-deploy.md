# 0002 — Never merge, never deploy

## Status
Accepted

## Context

If work starts without a human, something has to bound what it can do while
nobody is looking. The loop in `docs/vision.md` §1 only stays closed for as long
as somebody is willing to leave it running, and that willingness does not
survive one bad unattended action.

Three boundaries were on the table, differing in where the human sits rather
than in how careful the system is.

## Decision

Work terminates at a proposal a human reviews. Nothing merges, nothing deploys,
and this is not configurable — there is no setting, no per-project override and
no escape hatch.

The point of making it absolute rather than conditional is what it removes: no
credential capable of landing code or reaching production has to exist anywhere
in the system. The boundary is enforced by the absence of a capability, not by a
check that could be wrong.

Rejected: **a policy envelope**, where declared classes of change (dependency
bumps, lint fixes, test repairs) land unattended and everything else waits. It
lost because it makes "which class is this change?" a judgement the system has
to get right every time, silently, with no human downstream — and a
misclassification is exactly the failure that ends the willingness to run this
at all.

Rejected: **deferring to the target repository's CI gate**, letting anything
that passes land. Elegant where the gate is strict, and it keeps the trust
question in one place. It lost because the safety story would then be inherited
rather than owned: it is only ever as good as the weakest project added to the
instance, and it degrades silently when somebody relaxes a check for unrelated
reasons.

## Consequences

Throughput is capped by review. An overnight run produces proposals waiting in
the morning, not merged work, and a project where review is the bottleneck gets
no faster.

Cheap to reverse in the sense that the code has nothing to undo — but expensive
in the sense that reversing it means introducing credentials that do not
currently exist, which changes the threat model of every other component. The
absence is the feature.

**Amended by `docs/decisions/0009-jobs-hold-their-own-platform-credentials.md`.**
The paragraph above is no longer true as written. A job now holds a repository
credential, so this record's guarantee rests on that credential's scope — and
possibly on branch protection applied by the platform itself, if scope turns out
not to separate opening a pull request from merging one. The decision stands
unchanged; what defends it does not, and 0009 says how. The original wording is
left intact because "the absence is the feature" was the reasoning, and knowing
we gave it up deliberately is worth more than a tidy record.

Revisit if the reviewing itself becomes the thing worth automating. That is a
different product with a different trust story, and it should be argued from
scratch rather than as an amendment to this.

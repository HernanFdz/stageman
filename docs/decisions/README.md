# Decisions

One file per decision that governs tracked code: `NNNN-short-slug.md`, numbered
in the order taken. They are append-only — a superseded record stays and says
what replaced it, because the reasoning that was wrong is often more useful than
the reasoning that was right.

Records sit alongside this file, numbered from `0001`. The directory listing is
the index; there is deliberately no second copy of it here, because a list the
filesystem already maintains is exactly the kind that stops being true quietly.

A record without these three is an assertion, not a decision:

- **The rejected alternative.** What else was on the table and why it lost.
  Without it the next person re-litigates from scratch, usually reaching the
  same answer at full cost.
- **What it costs to reverse.** "~30 lines, well isolated" and "a data migration"
  are different decisions even when the reasoning is identical.
- **The revisit trigger.** What would make this wrong: a scale, a dependency, a
  deadline. "Correct at friend-group scale; revisit above ~10 writes/second"
  ages honestly. "Correct" does not.

## Shape

```markdown
# 0001 — Title

## Status
Accepted | Superseded by 0007

## Context
What forced a choice. Include the measurements if there were any.

## Decision
What was chosen, and what was rejected in its favour.

## Consequences
What this costs, what it forecloses, and what would make it wrong.
```

Keep them short. A record nobody reads protects nobody.

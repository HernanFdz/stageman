# 0072 — The dashboard has a dark theme

## Status

Accepted. Spends what
`docs/decisions/0026-the-dashboards-vocabulary-is-a-token-set.md` set aside:
that record named colours for their role so that a dark theme would be a
second block rather than an edit to every component, and this is the second
block.

## Context

The dashboard has one look, light, and the tokens in `app/tailwind.css` are
the only file that knows what any colour is. Four things get decided at once
by a second look, and only the first is aesthetic.

- **How a second set of values is expressed.** The same names with other
  values, or a variant on every component that needs one.
- **How the choice is made.** By the system alone, or by a person, with the
  system as the default.
- **Where the choice is kept.** There is no account and no user, per
  `docs/vision.md` §3, so anything kept on the server is kept for the
  instance rather than for whoever is looking at it.
- **When it is applied.** A page is rendered on the server and arrives
  complete, so a choice applied by the browser's half after it wakes shows
  the light page first and flashes.

## Decision

**The dark theme is the same tokens with other values, under one class on
the root; a person chooses light, dark, or the system, the choice is kept in
the browser, and a script in the head applies it before the first paint.**

- **One class on the root, one block of values.** A component never names a
  theme. One that needs to is one whose colour has no token yet, and the
  answer is the token, per 0026.
- **Three states.** System, light, dark, in that order of default. System
  means the page follows the platform's preference and changes with it;
  the other two hold.
- **Kept in the browser.** The choice is a fact about the eyes in front of
  the screen, not about the instance, and the browser's own storage is where
  such a fact belongs. A second browser starts on the system's preference,
  which is right.
- **Applied before paint.** A few lines of script in the head read the
  choice and set the class before the body is drawn, so a dark page is dark
  from its first frame. They also declare the colour scheme to the browser,
  so its own controls and scrollbars follow. It is the one script the
  dashboard writes by hand, and it stays a few lines.
- **The three state colours are chosen again for dark.** Working, idle and
  failed are read against the background, and a tint that separates them on
  white does not on near-black.

Rejected: **the system's preference alone.** No toggle, no script and no
storage, and no way for somebody who keeps a dark desk to read this one tool
light, which is the ordinary reason a toggle exists.

Rejected: **a variant on each component.** What 0026 exists to avoid: a
colour used directly is a colour found by text search when it changes, twice
over.

Rejected: **a preference on the server.** It needs somebody to belong to,
and the only somebody is the instance; and a page rendered from it would
still need the script to avoid the flash, so it buys nothing the browser's
storage does not.

## Consequences

**A second block in the stylesheet, and a rule.** Every token gets a dark
value, and a token added later without one is a component that turns
invisible at night, which is the review question for any change there.

**The page carries a script.** A few lines, inline, before the stylesheet.
Nothing else here runs script that was written rather than compiled, and
this record is the reason the exception exists.

**Tests read the page for the script and the class**, and the integration
test's page arrives with neither class set, because the server does not know
and the script decides.

**Reversing** is deleting a block, a control and the script.

**Revisit if** the dashboard ever gets an account, at which point the choice
could travel with the person; or if a third look is wanted, a high-contrast
one most likely, which is a third block under a third class and nothing more.

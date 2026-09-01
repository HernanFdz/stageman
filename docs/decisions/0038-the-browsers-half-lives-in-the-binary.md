# 0038 — The browser's half lives in the binary

## Status
Accepted. Answers the question `docs/open-questions.md` asks about whether the
browser's bundle should live inside the binary, and completes the deployment
story `docs/decisions/0035-an-image-is-built-never-named.md` began.

## Context

0035 put the agent's recipe inside the binary, on the grounds that a host needs
a container runtime and nothing else. What shipped was still two things: a
server executable and a `public/` directory beside it. Two artifacts can be
separated, paired with the wrong build, or copied without `-r`, and the failure
when they are is a page that renders and never comes alive — which looks almost
exactly like one that works.

Three things constrain the answer, and only the first was expected.

**Serving the assets ourselves is already supported.** `serve_api_application`
is documented for exactly this — it serves a Dioxus application *without* static
assets, "useful when static assets are served by another system" — and differs
from `serve_dioxus_application` by one line, the call mounting the directory.

**The index is not a static asset.** It is parsed once into fragments and
interleaved with the rendered page: everything before the title, the title,
everything after, the close of the head, the app, then two trailing pieces. It
is an input to the renderer, so serving it ourselves is not a substitute for
giving it to the framework.

**The framework accepts it only as a path.** The reachable constructor reads
the index from a directory derived from `DIOXUS_PUBLIC_PATH`, or from beside
the executable. There is a constructor taking a parsed index in memory, and its
own documentation says to build one with `IndexHtml::new` — but that type is
`pub(crate)` behind a private module, so nothing outside can name it, construct
it, or be handed one. Checked in 0.7.9, 0.7.10 and 0.8.0-alpha.1: identical in
all three.

That last one has a consequence which decides the mechanism. Redirecting the
framework to a directory of our choosing means setting an environment variable,
and in edition 2024 that call is `unsafe` — which this workspace forbids, and
forbids in a way no attribute can override. **So redirecting was never
available**, and the only remaining move is to comply.

**The path is read exactly once.** The function resolving that directory has
two callers: the one mounting static assets, which is the line
`serve_api_application` omits, and the configuration's constructor. The index
is parsed into an owned value and kept; nothing consults the path again.

## Decision

**The bundle is compiled in, and the index exists on disk only for the instant
it takes to parse it — at the path the framework already derives.**

- **A build script embeds the bundle**, emitting one entry per file. No
  dependency: what a crate would offer is a macro over a directory walk, and
  this is the walk.
- **At startup the index is written where the framework will look**, the
  configuration is built, and the file is removed — along with the directory it
  needed, if that is now empty. Nothing is on disk while the daemon runs. That
  location is computed by the same function that already answers "is there a
  bundle beside me", so the two can never disagree.

  Removing the directory is attempted rather than decided: `remove_dir` is
  `rmdir`, which takes an empty directory and refuses a full one in a single
  operation, so trying *is* the check and there is no window between them. A
  directory holding anything else belongs to somebody — an operator who pointed
  the variable at a real bundle — and being refused is the outcome asked for.
- **Everything else is served from memory**, one exact route per file, with
  `serve_api_application` so the framework never looks at a directory.
- **An absent embedded bundle reproduces the previous behaviour exactly.** A
  plain `cargo build` embeds nothing and falls back to looking beside the
  executable, which is what keeps `just dev`, `just check` and a fresh clone
  working without a `cfg` anywhere.

**The escape for an unwritable directory is the variable the framework already
reads.** Installed somewhere its own directory is read-only, the binary is told
`DIOXUS_PUBLIC_PATH` and writes there instead — one line in a service unit.
That is why complying with the framework's rule is better than redirecting it
even setting the unsafety aside: the operator gets a lever that already exists
and is already documented, rather than one this project invented.

Rejected: **setting the variable ourselves.** It is what makes a temporary
directory usable and it needs `unsafe`, so it would mean relaxing
`forbid(unsafe_code)` — a bar named in `AGENTS.md` — to buy a directory choice.
Complying costs a writability assumption instead, and that assumption is
already true of the deployment `docs/vision.md` §3 describes.

Rejected: **deferring to an index already at that path.** It sounds safer and
is the opposite. Assets are named by a hash of their contents, and this
binary's index names the hashes this binary carries, so an index left by
another build sends a browser after files nothing here has. Overwriting is the
safe direction.

Rejected: **owning the rendering.** The lower door is open — the calls
registering server functions and building a headless state are both public, so
the framework can be made not to touch the HTML at all. It takes the renderer
with it, and that is some eight hundred lines of hydration state, head
management and streaming.

Rejected: **a crate for embedding**, which would be audited, licence-checked
and pinned for ever in exchange for a loop. Rejected: **forking the framework**
to export one type, which is the trade
`docs/decisions/0034-tools-are-served-not-shipped.md` already refused.

## Consequences

**A release is two passes.** The server can only embed a bundle that exists,
and the tooling builds both halves in parallel — its one ordering flag runs the
*server* first, which is the wrong way round. Measured: no client-only build is
available either, since asking for the web platform alone still produces a
server. So `just build` runs, stages the client, and runs again.

**The bundle directory is emptied at the end of that recipe, and by
`just dev`.** It is a compile-time input, so a directory left full would make
an ordinary `cargo build` produce a different binary locally than it does in
continuous integration — a difference only ever noticed from the far side. A
tree at rest has it empty.

**The binary writes to its own directory at startup unless told otherwise**,
and cannot start if it may not. That is a refusal rather than a warning: a
binary carrying a browser half and unable to place its index would serve a page
that renders and never responds, and the dashboard is exactly the thing an
operator would otherwise be told to go and fix it in.

**One route per file is ours, and so are its content types.** A wrong type is
silent — a stylesheet served as text is ignored and a wasm module served as
anything else is refused — so the types are named rather than guessed, and
anything unrecognised is handed back as bytes.

**Exact routes rather than a wildcard**, so there is no path to normalise and
no way to ask for something outside the table. Directory traversal is absent
rather than defended against.

**A fresh clone still builds**, because the build script guarantees the
directory exists before anything reads it — the same mechanism and the same
reason as
`docs/decisions/0025-a-build-script-guarantees-the-stylesheet-exists.md`.

**Reversing** is deleting the route, the embedding and the write, and calling
the framework's own serving again — a few lines, no persisted state, nothing
that outlives the change.

**Revisit if** the framework exports its index type. That removes the write,
the file, the writability assumption and the refusal that guards it, all at
once — it is one line upstream and its own documentation already assumes it.
Revisit also if a client-first build appears, which collapses two passes into
one.

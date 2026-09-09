# The base every image this project builds starts from, and the first fragment
# of every recipe it composes.
#
# **A recipe is composed rather than written.** What reaches a runtime is this
# file, then the fragment installing one agent's adapter, then the fragment
# that holds a container open, and — for a job and not for a foreman — the
# fragment that reaches a repository. The pieces are here and the order is the
# agent crate's; see
# `docs/decisions/0051-an-image-is-named-by-the-recipe-it-is-built-from.md`.
#
# **The base is this project's rather than each adapter's**, which is the whole
# reason it is a fragment of its own. Everything composed after it assumes what
# is in here — a shell, Debian's package manager, the interpreter an adapter is
# installed by — so an adapter free to bring its own base would be an adapter
# that silently invalidates every fragment beside it and every container test
# with them. One base means those are established once. An adapter that cannot
# live on it changes this file for everybody, and the tests are taken again.
#
# Pinned deliberately. Both agents examined update themselves, so an unpinned
# base would let what runs change underneath a long-running instance between
# one job and the next — which is a large part of why images exist here at all.
#
# Deliberately no `# syntax=` directive. Nothing composed from these fragments
# uses a feature only the newer frontend provides, and asking for one costs a
# frontend image resolved over the network on a machine that has never built
# this — which is the machine that most needs the build to work.
#
# Every fragment is compiled into the binary with `include_str!` and built from
# standard input with no context, per
# `docs/decisions/0035-an-image-is-built-never-named.md`. A `COPY` would
# therefore not work, and there is nothing to copy: 0034 decided that nothing
# this project writes goes in the image.
FROM node:22-slim

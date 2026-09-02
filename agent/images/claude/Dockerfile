# Two images, one recipe. The stages below are the two things that run an
# agent — `docs/decisions/0036-a-foremans-image-is-not-a-jobs.md` — and the
# second is the first plus the layer that reaches a repository, so a foreman's
# container cannot run a repository tool because none is installed rather than
# because it was given no credential.
#
# Deliberately no `# syntax=` directive. Nothing here uses a feature only the
# newer frontend provides, and asking for one costs a frontend image resolved
# over the network on a machine that has never built this — which is the
# machine that most needs the build to work.
#
# This file is compiled into the binary with `include_str!` and built from
# standard input with no context, per
# `docs/decisions/0035-an-image-is-built-never-named.md`. A `COPY` would
# therefore not work, and there is nothing to copy: 0034 decided that nothing
# this project writes goes in the image.

# ---------------------------------------------------------------- the foreman
#
# What a project's foreman thinks with: the agent, and nothing else at all.
# It has no workspace and no repository, and its handout carries no platform
# credential, so everything in the next stage would be a capability with
# nothing to authenticate it.
FROM node:22-slim AS thinking

# Pinned deliberately. Both agents examined update themselves, so an unpinned
# image would let the agent change underneath a long-running instance between
# one job and the next — which is a large part of why images exist here at all.
#
# Note that this reaches the registry over HTTPS *before* the next stage
# installs a certificate bundle. That is not an oversight, it is the
# measurement: this base image ships no bundle, and the runtime brings its own
# roots, which is why the bundle below belongs to a job rather than to both.
RUN npm install --global --no-fund --no-audit \
        @agentclientprotocol/claude-agent-acp@0.70.0

# A job's workspace is this directory. A foreman's container never uses it and
# it costs nothing to leave here, where both stages can see it.
WORKDIR /workspace

# Something that does nothing and does not exit, so that the container outlives
# any one turn. The agent is run *inside* this container rather than being it —
# see `docs/decisions/0043-a-container-lives-as-long-as-its-tunnel-answers.md`,
# which needs a container able to keep serving after the agent that started
# something has stopped.
#
# Deliberately not a program this project writes, which is
# `docs/decisions/0034-tools-are-served-not-shipped.md`'s property and would
# have been the easy thing to give up here: a supervisor deciding the lifetime
# from inside would need the tunnel's port number, and a port number in an
# image is a port number that can be older than the instance driving it.
#
# It is not process one — the container is created with the runtime's own init,
# which reaps what an agent orphans and, just as usefully, passes on the signal
# that stops a container. Without that this ignores it, because process one has
# no default handlers, and every stop would wait for a timeout before being
# killed.
#
# `CMD` rather than `ENTRYPOINT`, and the difference is load-bearing rather
# than stylistic. A command named on the command line *replaces* this and is
# *appended* to an entry point — so with an entry point, the one-shot paths
# that still run the agent as the container would ask for `sleep infinity
# claude-agent-acp` and get a container that sleeps. Which is a container that
# starts perfectly and never speaks.
CMD ["sleep", "infinity"]

# -------------------------------------------------------------------- a job
#
# Everything above, plus the tools a job needs to reach a repository at all.
# Nothing outside the container delivers one — an agent that needs a repository
# clones it, per `docs/decisions/0016-the-agent-clones-the-repository.md` — so
# the tools to do that have to be in here, and installed at build time for the
# reason the agent is.
FROM thinking AS working

# Pinned for the reason the agent is pinned: an image that resolves a version
# at build time gives a different container to two jobs started a week apart,
# and the one that breaks is the one nobody changed. Debian's git is pinned
# already by the base image's own tag; the repository host's tool is not
# packaged there, so it is fetched at a named version.
ARG GH_VERSION=2.98.0
RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        ca-certificates \
        curl \
        git \
    && arch="$(dpkg --print-architecture)" \
    && curl --fail --silent --show-error --location \
        "https://github.com/cli/cli/releases/download/v${GH_VERSION}/gh_${GH_VERSION}_linux_${arch}.tar.gz" \
        --output /tmp/gh.tar.gz \
    && tar --extract --gzip --file /tmp/gh.tar.gz --directory /tmp \
    && install -m 0755 "/tmp/gh_${GH_VERSION}_linux_${arch}/bin/gh" /usr/local/bin/gh \
    && rm -rf /tmp/gh.tar.gz "/tmp/gh_${GH_VERSION}_linux_${arch}" \
    && rm -rf /var/lib/apt/lists/*

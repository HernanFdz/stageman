# syntax=docker/dockerfile:1

# The agent is installed at build time rather than provisioned at spawn:
# installing on every container start would put minutes in front of every
# signal. See docs/decisions/0012-agents-run-in-containers.md.
FROM node:22-slim

# Pinned deliberately. Both agents examined update themselves, so an unpinned
# image would let the agent change underneath a long-running orchestrator
# between one job and the next — which is a large part of why images exist here
# at all.
RUN npm install --global --no-fund --no-audit \
        @agentclientprotocol/claude-agent-acp@0.70.0

# The tools a job needs to reach a repository at all. Nothing outside the
# container delivers one — an agent that needs a repository clones it, per
# docs/decisions/0016-the-agent-clones-the-repository.md — so the tools to do
# that have to be in here, and installed at build time for the same reason the
# agent is.
#
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

# A job's workspace arrives here at run time. The orchestrator's own agent has
# none, because it has no repository to work on.
WORKDIR /workspace

# The protocol is JSON-RPC over standard input and output, so a container is
# driven exactly as a subprocess is: `docker run -i`, no ports, and no network
# needed for the channel itself.
ENTRYPOINT ["claude-agent-acp"]

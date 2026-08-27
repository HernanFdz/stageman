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

# A job's workspace arrives here at run time. The orchestrator's own agent has
# none, because it has no repository to work on.
WORKDIR /workspace

# The protocol is JSON-RPC over standard input and output, so a container is
# driven exactly as a subprocess is: `docker run -i`, no ports, and no network
# needed for the channel itself.
ENTRYPOINT ["claude-agent-acp"]

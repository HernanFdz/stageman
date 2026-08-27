//! The contract every coding agent is driven through, and the adapters that
//! implement it.
//!
//! Two shapes of use and one contract. A one-shot structured query is how the
//! orchestrator thinks; a long-running session in a workspace is how a job
//! works. Both reach a model only by running a configured agent, never through
//! a vendor's own service API — see
//! `docs/decisions/0007-model-work-goes-through-an-agent-cli.md` for why that
//! is a hard rule rather than a preference.
//!
//! Nothing outside an adapter may be specific to one agent. A change that
//! would make the contract fit one vendor more comfortably is the thing this
//! crate exists to catch — see `docs/decisions/0006-agents-are-pluggable.md`,
//! and note that the same record explains why this abstraction was refused
//! until now.
//!
//! The shape that contract takes was settled by a spike rather than by
//! argument; `docs/decisions/0010-acp-is-the-agent-contract.md` records the
//! choice, and rather more usefully, the evidence that outlives it.

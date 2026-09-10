//! The scenarios: the instance stepped against the simulated world, one
//! module per flow. One test binary, so that support one flow does not use
//! is not dead code in its binary and alive in another's.

#![expect(
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    reason = "test support in an integration-test crate is not seen as test code by the \
              lints' allowances, and a world that cannot open its own file has nothing to \
              report to; the virtual clock adds"
)]

mod foreman;
mod replies;
mod simulation;
mod waking;

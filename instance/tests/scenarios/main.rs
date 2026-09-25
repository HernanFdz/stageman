//! The scenarios: the instance stepped against the simulated world, one
//! module per flow. One test binary, so that support one flow does not use
//! is not dead code in its binary and alive in another's.

#![expect(
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    reason = "test support in an integration-test crate is not seen as test code by the \
              lints' allowances, and a world that cannot open its own file has nothing to \
              report to; the virtual clock adds"
)]

mod apps;
mod booting;
mod channel;
mod channel_apps;
mod checks;
mod credentials;
mod dashboard;
mod foreman;
mod foreman_room;
mod inbox;
mod installations;
mod listening;
mod notices;
mod replies;
mod signals;
mod simulation;
mod threads;
mod tools;
mod transcript;
mod tunnel;
mod turns;
mod waking;
mod workspaces;

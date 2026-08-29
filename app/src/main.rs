//! Running this is what running stageman means.
//!
//! **Two entry points, one binary.** `dx` compiles this same target twice —
//! once for the machine the daemon runs on and once for
//! `wasm32-unknown-unknown` — so which `main` exists is a feature selection,
//! and the daemon wins if somebody asks for both. Everything either one does
//! is in the library: this file is the fork and nothing else, which is the
//! only way both halves stay readable at a glance.
//!
//! It asks nothing. An instance starts with no agents, no projects and no
//! container runtime, and everything is configured through the dashboard —
//! `docs/decisions/0021-an-instance-starts-empty.md`. What used to be a
//! first-run flow, with a terminal prompt and an environment fallback for the
//! machines that have no terminal, is gone: there is nothing left to ask.
//!
//! One thing still comes from the environment, and it is the one that cannot
//! come from anywhere else: the key the instance's file is encrypted under,
//! which stored beside that file would defeat the encryption. Where the file
//! goes is derived from the platform, with a variable to override it.

/// The daemon: starts an instance, then serves its dashboard.
#[cfg(feature = "server")]
fn main() -> std::process::ExitCode {
    stageman::serve()
}

/// The browser: hydrates the page the daemon rendered.
///
/// It reaches the instance only through the server functions in
/// `stageman::dashboard`, which is the whole of what it is allowed to know —
/// see `docs/decisions/0022-the-browser-never-sees-the-domain.md`.
///
/// Skipped by mutation testing, and untestable rather than untested: the suite
/// runs under default features, which select the daemon, so this function is
/// not compiled at all while it runs. Every mutation of it therefore survives
/// for the same uninformative reason. Proving it needs a browser, and what
/// stands in for one today is `just dashboard`.
#[mutants::skip]
#[cfg(not(feature = "server"))]
fn main() {
    dioxus::launch(stageman::Dashboard);
}

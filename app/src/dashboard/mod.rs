//! What a page is allowed to know about an instance, and the routes it reads
//! through.
//!
//! **The only part of this crate compiled for both halves**, which is what
//! decides what may appear in it. Every type crossing the wire is plain and
//! serialisable, converted from the domain on the server and never the domain
//! itself — see `docs/decisions/0022-the-browser-never-sees-the-domain.md`.
//! What is in the browser is in the browser's hands, so what reaches it is
//! chosen rather than inherited, which is the same rule
//! `docs/decisions/0008-one-credential-per-agent.md` applies to an agent
//! process wearing different clothes.
//!
//! One module per screen, each holding its own routes beside the view that
//! reads them. That is a smaller arrangement than separating the two, and the
//! thing it optimises for is the question actually asked while working here —
//! *what does this screen need?* — rather than *what routes exist?*, which the
//! compiler can answer.

// Scoped to the browser's half, where the request a server function turns into
// is built from browser futures that are `!Send` by construction — there is
// one thread there, and nothing to send anything to.
//
// Here rather than on each function, which is where it belongs and where it
// does not survive: the server-function macro re-emits doc comments and drops
// every other attribute, so an expectation written on the item never reaches
// the code generated from it. The same attribute on the *server* side does
// work, because that half is the original function rather than something
// generated from it.
#![cfg_attr(
    not(feature = "server"),
    expect(
        clippy::future_not_send,
        reason = "there is one thread in a browser, and nothing to send it to"
    )
)]

mod agents_view;
mod error;
mod instance_view;

use dioxus::prelude::*;

pub use agents_view::{Agent, AgentsView};
pub use error::{DashboardError, DashboardResult};
pub use instance_view::{Instance, InstanceView, Project};

/// The dashboard's stylesheet.
///
/// Resolved at compile time, which is the only way this framework will serve a
/// file at all — assets are bundled because something referenced them, and a
/// directory of files nothing references is copied nowhere. It is also why
/// this crate has a build script: the file is Tailwind's output and therefore
/// absent on a fresh clone, so something has to guarantee it exists before the
/// macro looks. See
/// `docs/decisions/0025-a-build-script-guarantees-the-stylesheet-exists.md`.
// Fires inside the macro's own expansion, on a `&[u8]` this code never writes
// or names. Nothing here can be restructured to satisfy it.
#[expect(
    clippy::volatile_composites,
    reason = "raised against third-party macro output, not against anything written here"
)]
const STYLESHEET: Asset = asset!("/assets/styles.css");

/// Every screen there is.
///
/// Flat, and it should stay that way for as long as it can. This operates one
/// instance on one machine; a hierarchy of routes would be describing an
/// information architecture that does not exist yet.
#[derive(Debug, Clone, PartialEq, Eq, Routable)]
#[rustfmt::skip]
pub enum Route {
    #[layout(Shell)]
        #[route("/")]
        InstanceView {},

        #[route("/agents")]
        AgentsView {},
}

/// The whole dashboard.
///
/// What both halves of the binary start from: the daemon renders it, the
/// browser hydrates it. It is only the router, because everything a page has
/// in common belongs to [`Shell`] instead — a root that also drew a header
/// would put the frame outside the routing and make a screen without one
/// impossible to add.
#[component]
pub fn Dashboard() -> Element {
    rsx! {
        Router::<Route> {}
    }
}

/// The frame every screen is drawn in.
///
/// Holds the stylesheet as well as the navigation, so that a screen is only
/// ever its own contents and no view has to remember to bring the page with
/// it.
#[component]
pub fn Shell() -> Element {
    rsx! {
        document::Stylesheet { href: STYLESHEET }
        div { class: "min-h-screen bg-background font-sans text-foreground",
            header { class: "border-b border-border bg-surface",
                div { class: "mx-auto flex max-w-5xl items-baseline gap-6 px-6 py-4",
                    span { class: "text-base font-semibold tracking-tight", "stageman" }
                    nav { class: "flex items-baseline gap-4 text-sm",
                        NavLink { to: Route::InstanceView {}, "Instance" }
                        NavLink { to: Route::AgentsView {}, "Agents" }
                    }
                }
            }
            main { class: "mx-auto max-w-5xl px-6 py-6", Outlet::<Route> {} }
        }
    }
}

/// One entry in the navigation, which knows whether it is the current one.
///
/// Split out because the alternative is repeating the active-state comparison
/// at every entry, and a navigation whose highlight is wrong on one tab is
/// worse than one with no highlight at all.
#[component]
fn NavLink(to: Route, children: Element) -> Element {
    let here = use_route::<Route>() == to;
    let tone = if here {
        "text-foreground font-medium"
    } else {
        "text-muted-foreground hover:text-foreground"
    };

    rsx! {
        Link { to, class: "{tone} transition-colors", {children} }
    }
}

// ------------------------------------------------------- the server's half
//
// Shared by the routes above, and compiled only for the daemon. They live here
// rather than in one screen's module because two screens read the same
// instance, and the conversion from domain to wire is the thing this crate
// exists to keep in one place.

/// The agent named by a wire identifier.
///
/// The identifier is the *dashboard's* vocabulary rather than the domain's,
/// which is why this match lives here. Adding an agent to the domain stops
/// this compiling until somebody decides what the browser calls it — and a
/// wire name is a contract, so deciding it deliberately is the point.
///
/// # Errors
///
/// Fails if nothing is called that.
#[cfg(feature = "server")]
fn named(identifier: &str) -> DashboardResult<stageman_core::Agent> {
    match identifier {
        "claude" => Ok(stageman_core::Agent::Claude),
        _ => Err(DashboardError::UnknownAgent {
            name: identifier.to_owned(),
        }),
    }
}

/// What the browser calls an agent, and what to show for it.
#[cfg(feature = "server")]
const fn wire_name(agent: stageman_core::Agent) -> (&'static str, &'static str) {
    match agent {
        stageman_core::Agent::Claude => ("claude", "Claude"),
    }
}

/// The projects that would break if this agent were forgotten.
///
/// Names rather than identifiers, because an identifier means nothing to
/// whoever is reading the screen. A project that has somehow lost its entry is
/// skipped rather than named as a blank, which cannot happen through
/// `State::used_by` and is not worth a panic to prove.
#[cfg(feature = "server")]
fn dependents(state: &stageman_core::State, agent: stageman_core::Agent) -> Vec<String> {
    state
        .used_by(agent)
        .filter_map(|project| state.projects.get(&project))
        .map(|project| project.name.clone())
        .collect()
}

/// Every agent this build can run, as the browser sees them.
#[cfg(feature = "server")]
fn listed(state: &stageman_core::State) -> Vec<Agent> {
    stageman_core::Agent::ALL
        .iter()
        .map(|agent| {
            let (id, name) = wire_name(*agent);
            Agent {
                id: id.to_owned(),
                name: name.to_owned(),
                description: agent.description().to_owned(),
                configured: state.agents.contains_key(agent),
                used_by: dependents(state, *agent),
            }
        })
        .collect()
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::{DashboardError, dependents, listed, named, wire_name};
    use stageman_core::{Agent, AgentConfig, Project, ProjectId, Secret, State};
    use std::collections::{BTreeMap, BTreeSet};

    /// An instance watching one project, which names Claude for everything.
    fn watching(name: &str) -> State {
        State {
            agents: BTreeMap::from([(
                Agent::Claude,
                AgentConfig {
                    auth_token: Secret::new("not-a-real-credential".to_owned()),
                },
            )]),
            projects: BTreeMap::from([(
                ProjectId::from_uuid(uuid::Uuid::nil()),
                Project {
                    name: name.to_owned(),
                    repository: "https://example.invalid/repo".to_owned(),
                    orchestrator_agent: Agent::Claude,
                    job_agents: BTreeSet::from([Agent::Claude]),
                    credentials: BTreeMap::new(),
                    jobs: BTreeMap::new(),
                },
            )]),
        }
    }

    #[test]
    fn an_identifier_the_browser_sends_back_names_the_agent_it_came_from() {
        for agent in Agent::ALL {
            let (id, _) = wire_name(*agent);

            assert_eq!(named(id), Ok(*agent), "{id} did not round-trip");
        }
    }

    /// The set is closed, so anything else is a stale page or a hand-made
    /// request rather than something an operator can fix.
    #[test]
    fn an_identifier_naming_nothing_is_refused_rather_than_guessed() {
        assert_eq!(
            named("gpt"),
            Err(DashboardError::UnknownAgent {
                name: "gpt".to_owned()
            })
        );
    }

    /// Every agent is nameable and shows as something.
    #[test]
    fn every_agent_has_an_identifier_and_a_name() {
        for agent in Agent::ALL {
            let (id, name) = wire_name(*agent);

            assert!(!id.is_empty(), "{agent:?} has no identifier");
            assert!(!name.is_empty(), "{agent:?} has no name");
        }
    }

    /// The query the whole removal guard rests on.
    ///
    /// If this ever answers "nothing depends on it" when something does, an
    /// operator removes a credential their projects need and finds out at the
    /// next signal.
    #[test]
    fn a_project_naming_an_agent_is_named_as_depending_on_it() {
        let state = watching("aviary");

        assert_eq!(dependents(&state, Agent::Claude), vec!["aviary".to_owned()]);
    }

    #[test]
    fn nothing_depends_on_an_agent_when_there_are_no_projects() {
        let state = State::default();

        assert!(dependents(&state, Agent::Claude).is_empty());
    }

    /// The listing is of every agent, not of the configured ones.
    ///
    /// A screen that hid the unconfigured ones could not be used to configure
    /// one, which is the only thing that screen is for.
    #[test]
    fn every_agent_is_listed_whether_or_not_it_is_configured() {
        let empty = listed(&State::default());

        assert_eq!(empty.len(), Agent::ALL.len());
        assert!(empty.iter().all(|agent| !agent.configured));
    }

    #[test]
    fn a_listed_agent_carries_what_would_break_and_never_its_credential() {
        let listing = listed(&watching("aviary"));
        let claude = listing.first().expect("Claude is listed");

        assert!(claude.configured);
        assert_eq!(claude.used_by, vec!["aviary".to_owned()]);
        // Not a formality: this type is what the invariant in
        // `docs/architecture.md` §2 is enforced by, and a field added to it
        // would compile perfectly.
        let served = serde_json::to_string(&listing).expect("it serialises");
        assert!(!served.contains("not-a-real-credential"), "{served}");
    }
}

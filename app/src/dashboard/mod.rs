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

pub(crate) mod agents_view;
mod error;
mod instance_view;
mod jobs_view;
mod projects_view;

use dioxus::prelude::*;

pub use agents_view::{Agent, AgentsView};
pub use error::{DashboardError, DashboardResult};
pub use instance_view::{Instance, InstanceView};
pub use jobs_view::{Job, ProjectJobsView, Standing, Working};
pub use projects_view::{Project, ProjectsView};

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

/// The mark a browser puts in its tab strip.
///
/// Named rather than left to the default, and that is the whole of what it is
/// for: a page declaring no icon is one every browser then asks `/favicon.ico`
/// for, and nothing here serves that path — so the first thing this dashboard
/// did on arriving was put a 404 in the console of whoever opened it. Declaring
/// one is what stops the request, rather than answering it.
///
/// It is tracked rather than generated, unlike the stylesheet above, so there
/// is nothing for the build script to guarantee and no entry in
/// `.quality/generated-paths`.
// The same expectation as the stylesheet, for the same reason: it fires inside
// the macro's expansion rather than against anything written here.
#[expect(
    clippy::volatile_composites,
    reason = "raised against third-party macro output, not against anything written here"
)]
const FAVICON: Asset = asset!("/assets/favicon.svg");

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

        #[route("/projects")]
        ProjectsView {},

        #[route("/projects/:project")]
        ProjectJobsView { project: String },
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
        document::Link { rel: "icon", r#type: "image/svg+xml", href: FAVICON }
        document::Stylesheet { href: STYLESHEET }
        div { class: "min-h-screen bg-background font-sans text-foreground",
            header { class: "border-b border-border bg-surface",
                div { class: "mx-auto flex max-w-5xl items-baseline gap-6 px-6 py-4",
                    span { class: "text-base font-semibold tracking-tight", "stageman" }
                    nav { class: "flex items-baseline gap-4 text-sm",
                        NavLink { to: Route::InstanceView {}, "Instance" }
                        NavLink { to: Route::AgentsView {}, "Agents" }
                        NavLink { to: Route::ProjectsView {}, "Projects" }
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
pub(crate) const fn wire_name(agent: stageman_core::Agent) -> (&'static str, &'static str) {
    match agent {
        stageman_core::Agent::Claude => ("claude", "Claude"),
    }
}

/// The identifier a browser sent back, as this instance knows it.
///
/// Compared as text rather than parsed, so that a malformed identifier and an
/// unknown one are the same answer — which they are, from the operator's side.
///
/// # Errors
///
/// Fails if nothing is watched under it.
#[cfg(feature = "server")]
fn identify(
    state: &stageman_core::State,
    identifier: &str,
) -> DashboardResult<stageman_core::ProjectId> {
    state
        .projects
        .keys()
        .find(|known| known.to_string() == identifier)
        .copied()
        .ok_or_else(|| DashboardError::UnknownProject {
            id: identifier.to_owned(),
        })
}

/// What a screen calls an agent.
#[cfg(feature = "server")]
fn shown(agent: stageman_core::Agent) -> String {
    wire_name(agent).1.to_owned()
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

/// What the browser calls a platform.
#[cfg(feature = "server")]
const fn wire_platform(platform: stageman_core::Platform) -> &'static str {
    match platform {
        stageman_core::Platform::GitHub => "github",
    }
}

/// What a screen calls a channel.
///
/// Deliberately one-directional, as every name on this screen now is. Nothing
/// sends a channel or a platform identifier back: the form binds the one
/// channel there is and sets the one platform there is, rather than choosing
/// among them. A platform once had a parser, for a route that set a credential
/// on its own; `amend` replaced that route and the parser went with it, which
/// is the rule this comment always stated — a parser with no caller is a guess
/// about a route that does not exist. Adding a second channel stops this
/// compiling until somebody names it, which is the property worth having.
#[cfg(feature = "server")]
const fn wire_channel(channel: stageman_core::Channel) -> &'static str {
    match channel {
        stageman_core::Channel::Slack => "Slack",
    }
}

/// One project, as the browser sees it.
#[cfg(feature = "server")]
fn projected(id: stageman_core::ProjectId, project: &stageman_core::Project) -> Project {
    Project {
        id: id.to_string(),
        name: project.name.clone(),
        repository: project.repository.clone(),
        // Identifiers rather than the names a person reads, and the difference
        // is load-bearing rather than cosmetic. These are what a browser sends
        // back when a project is changed, and `named` accepts only the
        // identifier — so carrying the display name here would produce a form
        // that renders perfectly, selects nothing, and is refused on submit
        // with "no agent called Claude". Rendering is the screen's job, and it
        // already holds the list that maps one to the other.
        foreman: wire_name(project.foreman_agent).0.to_owned(),
        job_agents: project
            .job_agents
            .iter()
            .map(|agent| wire_name(*agent).0.to_owned())
            .collect(),
        // Which platforms have one, never what it is. There is nowhere on this
        // type to put a credential, which is the point.
        platforms: project
            .credentials
            .keys()
            .map(|platform| wire_platform(*platform).to_owned())
            .collect(),
        // Which channels are bound, never the address or the credential. The
        // address is not a secret, but this type exists to carry what a screen
        // needs, and no screen needs it yet — see
        // `docs/decisions/0022-the-browser-never-sees-the-domain.md`.
        channels: project
            .channels
            .keys()
            .map(|channel| wire_channel(*channel).to_owned())
            .collect(),
        working: project
            .jobs
            .values()
            .filter(|job| job.progress == stageman_core::Progress::Working)
            .count(),
        jobs: project.jobs.len(),
    }
}

/// Every project this instance watches.
#[cfg(feature = "server")]
fn watching(state: &stageman_core::State) -> Vec<Project> {
    state
        .projects
        .iter()
        .map(|(id, project)| projected(*id, project))
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
    use super::{
        DashboardError, dependents, identify, listed, named, shown, wire_channel, wire_name,
        wire_platform,
    };
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
                    foreman_agent: Agent::Claude,
                    job_agents: BTreeSet::from([Agent::Claude]),
                    credentials: BTreeMap::new(),
                    channels: BTreeMap::new(),
                    jobs: BTreeMap::new(),
                    variables: BTreeMap::new(),
                    attending: stageman_core::Attending::default(),
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
    /// What a platform is called on the wire, asserted as the literal text.
    ///
    /// This was a round trip until `amend` subsumed the route that parsed one
    /// back. With no parser left, the literal is the whole of the contract —
    /// the same position `wire_channel` below has always been in, and asserted
    /// the same way so that emptying it or replacing it with nonsense fails.
    ///
    /// Note it is an identifier and not a display name, which is the one place
    /// this differs from a channel: nothing renders a platform, so nothing
    /// ever needed the other half. Asserting the literal is what says which of
    /// the two it is.
    ///
    /// One platform, so written out rather than looped. It becomes a loop over
    /// the set when there is a second, in the same way the agent tests above
    /// already are.
    #[test]
    fn a_platform_is_named_on_the_wire_by_something_that_says_something() {
        let id = wire_platform(stageman_core::Platform::GitHub);

        assert!(!id.is_empty());
        assert_eq!(id, "github");
    }

    /// What a channel is called on a screen, asserted as the literal text.
    ///
    /// Not a round trip like the platform above, because there is deliberately
    /// nothing to round trip through: nothing sends a channel identifier back,
    /// so `wire_channel` has no parser and this name is only ever read. That
    /// makes the name itself the whole of the contract — and mutation testing
    /// is what said so, by replacing it with an empty string and with nonsense
    /// and finding no test that minded.
    ///
    /// One channel, so written out rather than looped, on the same terms as
    /// the platform above.
    #[test]
    fn a_channel_is_shown_by_a_name_that_says_something() {
        let name = wire_channel(stageman_core::Channel::Slack);

        assert!(!name.is_empty());
        assert_eq!(name, "Slack");
    }

    /// What a screen calls an agent is what a refusal has to name.
    #[test]
    fn an_agent_is_shown_by_the_name_the_screen_uses() {
        for agent in Agent::ALL {
            assert_eq!(shown(*agent), wire_name(*agent).1);
            assert!(!shown(*agent).is_empty());
        }
    }

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

    /// An identifier the browser sends back finds the project it came from.
    #[test]
    fn an_identifier_finds_the_project_it_names() {
        let id = stageman_core::ProjectId::from_uuid(uuid::Uuid::from_u128(9));
        let mut state = watching("aviary");
        let project = state
            .projects
            .values()
            .next()
            .cloned()
            .expect("the project");
        state.projects.clear();
        state.projects.insert(id, project);

        assert_eq!(identify(&state, &id.to_string()), Ok(id));
    }

    /// Anything else is not found rather than matched to whatever is nearest.
    #[test]
    fn an_identifier_naming_nothing_finds_nothing() {
        let state = watching("aviary");
        let other = stageman_core::ProjectId::from_uuid(uuid::Uuid::from_u128(10)).to_string();

        assert_eq!(
            identify(&state, &other),
            Err(DashboardError::UnknownProject { id: other.clone() })
        );
        assert!(identify(&state, "not-an-identifier").is_err());
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

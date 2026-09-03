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
pub use projects_view::{Choice, Fitted, KitDraft, ModelChoice, Project, ProjectsView, Shape};

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

/// What the browser calls one of Claude's models, and what to show for it.
///
/// The browser's vocabulary rather than the adapter's, which spells the same
/// models for the wire in the agent crate. The two happen to agree today and
/// are separate contracts: one is what a page sends back, the other is what a
/// pinned adapter accepts, and a change to either is a change to one.
#[cfg(feature = "server")]
const fn wire_model(model: stageman_core::ClaudeModel) -> (&'static str, &'static str) {
    match model {
        stageman_core::ClaudeModel::Default { .. } => ("default", "Default"),
        stageman_core::ClaudeModel::Sonnet { .. } => ("sonnet", "Sonnet"),
        stageman_core::ClaudeModel::Opus { .. } => ("opus", "Opus"),
        stageman_core::ClaudeModel::Haiku => ("haiku", "Haiku"),
    }
}

/// What the browser calls an effort level, and what to show for it.
#[cfg(feature = "server")]
const fn wire_effort(effort: stageman_core::ClaudeEffort) -> (&'static str, &'static str) {
    match effort {
        stageman_core::ClaudeEffort::Default => ("default", "Default"),
        stageman_core::ClaudeEffort::Low => ("low", "Low"),
        stageman_core::ClaudeEffort::Medium => ("medium", "Medium"),
        stageman_core::ClaudeEffort::High => ("high", "High"),
        stageman_core::ClaudeEffort::XHigh => ("xhigh", "Extra high"),
        stageman_core::ClaudeEffort::Max => ("max", "Max"),
    }
}

/// One of each of Claude's models, for enumerating what a browser may choose.
///
/// The effort on the ones that carry one is a placeholder: what this list is
/// for is the *kind* of model, and [`kit_of`] puts the effort a browser asked
/// for in its place.
#[cfg(feature = "server")]
const CLAUDE_MODELS: [stageman_core::ClaudeModel; 4] = [
    stageman_core::ClaudeModel::Default {
        effort: stageman_core::ClaudeEffort::Default,
    },
    stageman_core::ClaudeModel::Sonnet {
        effort: stageman_core::ClaudeEffort::Default,
    },
    stageman_core::ClaudeModel::Opus {
        effort: stageman_core::ClaudeEffort::Default,
    },
    stageman_core::ClaudeModel::Haiku,
];

/// A kit, as a browser edits it: identifiers for the agent, the model, and
/// the effort where the model has one.
#[cfg(feature = "server")]
pub(crate) fn fitted(kit: &stageman_core::Kit) -> Fitted {
    match kit {
        stageman_core::Kit::Claude { model } => Fitted {
            agent: wire_name(stageman_core::Agent::Claude).0.to_owned(),
            model: wire_model(*model).0.to_owned(),
            effort: model
                .effort()
                .map_or_else(String::new, |effort| wire_effort(effort).0.to_owned()),
        },
    }
}

/// The kit a browser described, if every part of it is one this build knows.
///
/// Refuses rather than mends: a model this build does not know is not mapped
/// onto the nearest one, and an effort asked of a model that has none is not
/// dropped, because the domain cannot hold that combination and a form saying
/// one thing while the kit does another is the failure
/// `docs/decisions/0048-a-job-runs-on-a-kit.md` exists to rule out.
///
/// # Errors
///
/// Fails if the agent, the model or the effort is not one this build knows,
/// if a model that takes an effort was given none, or if one that takes none
/// was given one.
#[cfg(feature = "server")]
pub(crate) fn kit_of(fitted: &Fitted) -> DashboardResult<stageman_core::Kit> {
    match named(&fitted.agent)? {
        stageman_core::Agent::Claude => {
            let kind = CLAUDE_MODELS
                .iter()
                .copied()
                .find(|model| wire_model(*model).0 == fitted.model)
                .ok_or_else(|| DashboardError::UnknownSetting {
                    field: "model".to_owned(),
                    value: fitted.model.clone(),
                })?;
            let effort = || -> DashboardResult<stageman_core::ClaudeEffort> {
                if fitted.effort.is_empty() {
                    return Err(DashboardError::Incomplete {
                        field: "effort".to_owned(),
                    });
                }
                stageman_core::ClaudeEffort::ALL
                    .iter()
                    .copied()
                    .find(|effort| wire_effort(*effort).0 == fitted.effort)
                    .ok_or_else(|| DashboardError::UnknownSetting {
                        field: "effort".to_owned(),
                        value: fitted.effort.clone(),
                    })
            };
            let model = match kind {
                stageman_core::ClaudeModel::Haiku => {
                    if !fitted.effort.is_empty() {
                        return Err(DashboardError::EffortNotOnModel {
                            model: wire_model(kind).1.to_owned(),
                        });
                    }
                    stageman_core::ClaudeModel::Haiku
                }
                stageman_core::ClaudeModel::Default { .. } => {
                    stageman_core::ClaudeModel::Default { effort: effort()? }
                }
                stageman_core::ClaudeModel::Sonnet { .. } => {
                    stageman_core::ClaudeModel::Sonnet { effort: effort()? }
                }
                stageman_core::ClaudeModel::Opus { .. } => {
                    stageman_core::ClaudeModel::Opus { effort: effort()? }
                }
            };
            Ok(stageman_core::Kit::Claude { model })
        }
    }
}

/// What one agent can be set to, as the choices a form offers.
///
/// Built here from the domain's closed sets because the browser's half cannot
/// name them — `docs/decisions/0022-the-browser-never-sees-the-domain.md` —
/// and a form that hard-coded them would be a second copy of the enumeration
/// that nothing holds to the first. The first model and the first effort are
/// the agent's defaults, which is what a new kit starts on.
#[cfg(feature = "server")]
pub(crate) fn shape_of(agent: stageman_core::Agent) -> Shape {
    match agent {
        stageman_core::Agent::Claude => Shape {
            agent: wire_name(agent).0.to_owned(),
            models: CLAUDE_MODELS
                .iter()
                .map(|model| {
                    let (id, name) = wire_model(*model);
                    ModelChoice {
                        id: id.to_owned(),
                        name: name.to_owned(),
                        has_effort: model.effort().is_some(),
                    }
                })
                .collect(),
            efforts: stageman_core::ClaudeEffort::ALL
                .iter()
                .map(|effort| {
                    let (id, name) = wire_effort(*effort);
                    Choice {
                        id: id.to_owned(),
                        name: name.to_owned(),
                    }
                })
                .collect(),
        },
    }
}

/// A kit in the words a person reads on a job's row: the agent, and whatever
/// differs from the agent's own defaults.
///
/// The defaults are left unsaid so that the ordinary job reads as it always
/// did, and a job on something other than the defaults says so.
#[cfg(feature = "server")]
pub(crate) fn described(kit: &stageman_core::Kit) -> String {
    match kit {
        stageman_core::Kit::Claude { model } => {
            let mut parts = vec![wire_name(stageman_core::Agent::Claude).1.to_owned()];
            if !matches!(model, stageman_core::ClaudeModel::Default { .. }) {
                parts.push(wire_model(*model).1.to_owned());
            }
            if let Some(effort) = model.effort()
                && effort != stageman_core::ClaudeEffort::Default
            {
                parts.push(wire_effort(effort).1.to_lowercase());
            }
            parts.join(" · ")
        }
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
        foreman: fitted(&project.foreman_kit),
        // Whole, name and description and settings, because this is what the
        // form edits and it has to open showing what is true. A kit holds no
        // credential, so there is nothing here to withhold.
        kits: project
            .kits
            .iter()
            .map(|(name, offered)| KitDraft {
                name: name.to_string(),
                description: offered.description.clone(),
                fitted: fitted(&offered.kit),
            })
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
        // Names, never values — there is nowhere on this type to put one. The
        // names are what an edit form shows, which is why they cross the wire
        // and a channel's address does not.
        variables: project
            .variables
            .keys()
            .map(std::string::ToString::to_string)
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
        DashboardError, Fitted, dependents, described, fitted, identify, kit_of, listed, named,
        shape_of, shown, wire_channel, wire_name, wire_platform,
    };
    use stageman_core::{
        Agent, AgentConfig, ClaudeEffort, ClaudeModel, Kit, KitConfig, KitName, Project, ProjectId,
        Secret, State,
    };
    use std::collections::BTreeMap;

    /// Every kit the domain can spell for Claude.
    fn every_claude_kit() -> Vec<Kit> {
        let mut kits = vec![Kit::Claude {
            model: ClaudeModel::Haiku,
        }];
        for effort in ClaudeEffort::ALL.iter().copied() {
            for model in [
                ClaudeModel::Default { effort },
                ClaudeModel::Sonnet { effort },
                ClaudeModel::Opus { effort },
            ] {
                kits.push(Kit::Claude { model });
            }
        }
        kits
    }

    /// Every kit crosses to the browser and back as itself.
    ///
    /// The whole grid rather than a sample, because a spelling that did not
    /// round-trip would send a job on a kit other than the one the operator
    /// saw on the form — the failure the read-back on the adapter's side
    /// exists to catch, arriving from the other direction.
    #[test]
    fn every_kit_survives_the_trip_to_the_browser_and_back() {
        let kits = every_claude_kit();
        assert_eq!(
            kits.len(),
            1 + 3 * ClaudeEffort::ALL.len(),
            "the whole grid, or the loop below proves nothing"
        );
        for kit in kits {
            assert_eq!(kit_of(&fitted(&kit)), Ok(kit.clone()), "{kit:?}");
        }
    }

    /// The form's shape says which models take an effort, and only Haiku
    /// does not.
    #[test]
    fn the_shape_of_claude_offers_an_effort_on_every_model_but_haiku() {
        let shape = shape_of(Agent::Claude);
        assert_eq!(shape.agent, "claude");
        let without: Vec<&str> = shape
            .models
            .iter()
            .filter(|model| !model.has_effort)
            .map(|model| model.id.as_str())
            .collect();
        assert_eq!(without, vec!["haiku"]);
        assert_eq!(
            shape.models.first().map(|model| model.id.as_str()),
            Some("default"),
            "a new kit starts on the agent's defaults, which come first"
        );
        assert_eq!(
            shape.efforts.first().map(|effort| effort.id.as_str()),
            Some("default")
        );
        assert_eq!(shape.efforts.len(), ClaudeEffort::ALL.len());
    }

    /// What the domain cannot hold is refused rather than mended.
    #[test]
    fn a_kit_a_browser_describes_badly_is_refused_rather_than_mended() {
        let claude = |model: &str, effort: &str| Fitted {
            agent: "claude".to_owned(),
            model: model.to_owned(),
            effort: effort.to_owned(),
        };

        assert_eq!(
            kit_of(&claude("haiku", "high")),
            Err(DashboardError::EffortNotOnModel {
                model: "Haiku".to_owned()
            }),
            "haiku offers no effort, so one asked of it is refused, not dropped"
        );
        assert_eq!(
            kit_of(&claude("opus", "")),
            Err(DashboardError::Incomplete {
                field: "effort".to_owned()
            }),
            "a model that takes an effort must be given one"
        );
        assert_eq!(
            kit_of(&claude("gpt-5", "high")),
            Err(DashboardError::UnknownSetting {
                field: "model".to_owned(),
                value: "gpt-5".to_owned()
            })
        );
        assert_eq!(
            kit_of(&claude("opus", "ultra")),
            Err(DashboardError::UnknownSetting {
                field: "effort".to_owned(),
                value: "ultra".to_owned()
            })
        );
        assert_eq!(
            kit_of(&Fitted {
                agent: "gpt".to_owned(),
                model: "default".to_owned(),
                effort: "default".to_owned(),
            }),
            Err(DashboardError::UnknownAgent {
                name: "gpt".to_owned()
            })
        );
    }

    /// A job's row names the agent and only what differs from its defaults.
    #[test]
    fn a_kit_is_described_by_what_differs_from_the_defaults() {
        assert_eq!(described(&Kit::defaults(Agent::Claude)), "Claude");
        assert_eq!(
            described(&Kit::Claude {
                model: ClaudeModel::Opus {
                    effort: ClaudeEffort::XHigh,
                },
            }),
            "Claude · Opus · extra high"
        );
        assert_eq!(
            described(&Kit::Claude {
                model: ClaudeModel::Haiku,
            }),
            "Claude · Haiku"
        );
        assert_eq!(
            described(&Kit::Claude {
                model: ClaudeModel::Default {
                    effort: ClaudeEffort::Low,
                },
            }),
            "Claude · low",
            "the default model is unsaid even when the effort is not"
        );
    }

    /// An instance watching one project, which names Claude for everything.
    #[expect(
        clippy::expect_used,
        reason = "a fixture kit that cannot be named is a broken test, and should say so"
    )]
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
                    foreman_kit: Kit::defaults(Agent::Claude),
                    kits: BTreeMap::from([(
                        KitName::new("Claude").expect("a name"),
                        KitConfig::defaults(Agent::Claude),
                    )]),
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

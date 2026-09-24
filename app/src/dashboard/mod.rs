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
mod env_file;
mod error;
mod home_view;
mod instance_view;
mod job_view;
mod jobs_view;
mod live;
mod project_settings_view;
mod projects_view;
mod status_view;

use dioxus::prelude::*;

use crate::ui::{THEME_SCRIPT, ThemeToggle};

pub use agents_view::{Agent, AgentsView};
pub use error::{DashboardError, DashboardResult};
pub use home_view::{Home, HomeView, ProjectJob};
pub use instance_view::{Apps, InstanceView, PlatformAppView, Registration};
pub use job_view::{JobPage, ProjectJobView};
pub use jobs_view::{Job, ProjectJobsView, Standing, Working};
pub use live::{Live, LiveMark};
pub use project_settings_view::{ProjectNewView, ProjectSettingsView};
pub use projects_view::{Choice, Fitted, KitDraft, ModelChoice, Project, ProjectsView, Shape};
pub use status_view::{Instance, Status};

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
        HomeView {},

        #[route("/agents")]
        AgentsView {},

        // What the instance configures about itself, after Agents in the
        // order a new instance is set up — see
        // `docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`.
        #[route("/instance")]
        InstanceView {},

        #[route("/projects")]
        ProjectsView {},

        // A static segment, which the router prefers to the dynamic one
        // below, so a project can never be called `new` by mistake — see
        // `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`.
        #[route("/projects/new")]
        ProjectNewView {},

        #[route("/projects/:project")]
        ProjectJobsView { project: String },

        #[route("/projects/:project/settings")]
        ProjectSettingsView { project: String },

        // A job's page keeps the identifier, since names are not unique —
        // `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`.
        #[route("/projects/:project/jobs/:job")]
        ProjectJobView { project: String, job: String },
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
/// it. It is also what keeps a page live: the stream of ticks is opened here,
/// once, and every screen's read follows it — see
/// `docs/decisions/0071-a-page-learns-of-change-from-a-tick.md`.
#[component]
pub fn Shell() -> Element {
    let live = use_context_provider(Live::new);
    live::use_live(live);

    rsx! {
        document::Link { rel: "icon", r#type: "image/svg+xml", href: FAVICON }
        // Told to the browser as well as decided by the script below, so that
        // its own controls and scrollbars follow the look.
        document::Meta { name: "color-scheme", content: "light dark" }
        // Before the stylesheet, so the class the dark tokens hang off is on
        // the root before the first rule applies, and a dark page is dark from
        // its first frame — see
        // `docs/decisions/0072-the-dashboard-has-a-dark-theme.md`.
        document::Script { "{THEME_SCRIPT}" }
        document::Stylesheet { href: STYLESHEET }
        // A column as tall as the window, so the status line sits at its
        // foot whatever a page's height, rather than wherever the contents
        // happened to end.
        div { class: "flex min-h-screen flex-col bg-background font-sans text-foreground",
            // In view from wherever a person has scrolled to, with the
            // status line at the foot the same way — `docs/conventions.md`
            // §3 — and above what scrolls under it, below the modal.
            header { class: "sticky top-0 z-40 border-b border-border bg-surface",
                div { class: "mx-auto flex max-w-5xl items-baseline gap-6 px-6 py-4",
                    span { class: "text-base font-semibold tracking-tight", "stageman" }
                    // In the order
                    // `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`
                    // gives: agents last, and still here, because they are
                    // the first step on a new instance.
                    nav { class: "flex items-baseline gap-4 text-sm",
                        NavLink { to: Route::HomeView {}, "Home" }
                        NavLink { to: Route::ProjectsView {}, "Projects" }
                        NavLink { to: Route::AgentsView {}, "Agents" }
                        NavLink { to: Route::InstanceView {}, "Instance" }
                    }
                    div { class: "ml-auto flex items-center gap-4 self-center",
                        LiveMark { live }
                        ThemeToggle {}
                    }
                }
            }
            main { class: "mx-auto w-full max-w-5xl flex-1 px-6 py-6", Outlet::<Route> {} }
            // The machine and the build, under every page rather than on one
            // of their own — see the same record.
            Status {}
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

/// Asks the instance something on a person's behalf, and hands back what it
/// answered — or what it refused with.
///
/// Every server function goes through this: one request in, one response
/// out, and the instance's refusal already in the shape the browser reads.
/// What is not a refusal and not the answer the route expected is a fault in
/// this process, which [`unexpected`] says.
#[cfg(feature = "server")]
pub(crate) async fn ask(
    request: stageman_instance::Request,
) -> DashboardResult<stageman_instance::Response> {
    let Some(asking) = crate::asking() else {
        return Err(DashboardError::NoInstance);
    };
    match asking.ask(request).await {
        Some(stageman_instance::Response::Refused(refusal)) => Err(refusal.into()),
        Some(response) => Ok(response),
        None => {
            tracing::error!("the instance did not answer a dashboard request");
            Err(DashboardError::Failed)
        }
    }
}

/// An answer of the wrong kind for the route that asked.
///
/// Cannot happen while every request is answered with its own kind of
/// response, and reported rather than assumed impossible.
#[cfg(feature = "server")]
pub(crate) fn unexpected(response: &stageman_instance::Response) -> DashboardError {
    tracing::error!(
        ?response,
        "the instance answered a request with the wrong kind of answer"
    );
    DashboardError::Failed
}

//! The projects an instance watches, and what each one needs in order to work.
//!
//! The screen that makes an instance able to *do* something. It comes after
//! agents because a project names one agent for its foreman and a
//! non-empty set its jobs may use, and both have to be configured before a
//! project may name them — `docs/decisions/0021-an-instance-starts-empty.md`.
//!
//! **Validity is asked, not restated.** What makes an instance valid is
//! `State::check` and nothing else; this screen builds the state it would
//! produce, asks, and reports the answer. Re-deciding here would be a second
//! definition of valid that could drift from the first, which is the trap 0021
//! chose a single function to avoid.

use dioxus::prelude::*;
#[cfg(feature = "server")]
use stageman_instance::{Request, Response};

use super::agents_view::Agent;
use super::error::{DashboardError, DashboardResult};
use crate::ui::{Badge, BadgeTone, Button, ButtonVariant, Card, EmptyState, Modal};

pub use stageman_wire::{
    ChannelDraft, Choice, Draft, Filling, Fitted, KitDraft, ModelChoice, Project, Shape,
    VariableDraft, Watching,
};

/// An agent as it comes: the shape's first model, and its first effort where
/// that model takes one.
///
/// What a new kit starts on, and what a fitted agent moves to when its agent
/// changes — nothing carries over between agents, because a model is one
/// agent's and not another's.
fn seeded(shape: &Shape) -> Fitted {
    let model = shape.models.first();
    Fitted {
        agent: shape.agent.clone(),
        model: model.map(|model| model.id.clone()).unwrap_or_default(),
        effort: model
            .filter(|model| model.has_effort)
            .and_then(|_| shape.efforts.first())
            .map(|effort| effort.id.clone())
            .unwrap_or_default(),
    }
}

/// The shape describing an agent, if the server sent one for it.
///
/// A function rather than a `find` at each of its three call sites, so that the
/// comparison it turns on is tested once — mutation testing inverted it inside
/// the component, where nothing could notice.
fn shape_for<'a>(shapes: &'a [Shape], agent: &str) -> Option<&'a Shape> {
    shapes.iter().find(|shape| shape.agent == agent)
}

/// Whether a model takes an effort, as its agent's shape says.
///
/// False for a model the shape does not list at all, which the form cannot
/// produce and a request written by hand can: the far side refuses it by name,
/// and offering an effort select for it here would be offering a second thing
/// to refuse.
fn takes_effort(shape: &Shape, model: &str) -> bool {
    shape
        .models
        .iter()
        .any(|choice| choice.id == model && choice.has_effort)
}

/// A fitted agent moved to another agent.
///
/// That agent's defaults, or — for an identifier no shape describes, which the
/// form cannot produce — the identifier alone, so that the refusal on the far
/// side names it rather than a substitute.
fn with_agent(shapes: &[Shape], agent: &str) -> Fitted {
    shape_for(shapes, agent).map_or_else(
        || Fitted {
            agent: agent.to_owned(),
            ..Fitted::default()
        },
        seeded,
    )
}

/// A fitted agent moved to another of its models.
///
/// The effort is kept where the new model takes one, cleared where it does
/// not, and given the first where the old model had none — so what the form
/// shows is always something the far side will accept.
fn with_model(fitted: &Fitted, shape: &Shape, model: &str) -> Fitted {
    let effort = if !takes_effort(shape, model) {
        String::new()
    } else if fitted.effort.is_empty() {
        shape
            .efforts
            .first()
            .map(|effort| effort.id.clone())
            .unwrap_or_default()
    } else {
        fitted.effort.clone()
    };
    Fitted {
        agent: fitted.agent.clone(),
        model: model.to_owned(),
        effort,
    }
}

/// Everything the projects screen shows.
///
/// # Errors
///
/// Fails if this process is not operating an instance.
#[get("/api/projects")]
pub async fn projects() -> DashboardResult<Watching> {
    match super::ask(Request::Projects).await? {
        Response::Projects(watching) => Ok(watching),
        other => Err(super::unexpected(&other)),
    }
}

/// Starts watching a repository.
///
/// # Errors
///
/// Fails if anything required is missing, if a kit describes settings this
/// build does not know, if the channel is half given, if another project
/// already listens where this one would, or if the instance would not be
/// consistent with this project in it.
#[post("/api/projects/create")]
pub async fn create(draft: Draft) -> DashboardResult<Watching> {
    match super::ask(Request::Create { draft }).await? {
        Response::Projects(watching) => Ok(watching),
        other => Err(super::unexpected(&other)),
    }
}

/// Changes what a project is, leaving what it has done alone.
///
/// A blank credential means the one it already has, never none: there is
/// nowhere on the wire for the current value, so the box always starts empty.
/// The channel is not offered at all, for the same reason.
///
/// # Errors
///
/// Fails as [`create`] does, and if nothing is watched under that identifier.
#[post("/api/projects/amend")]
pub async fn amend(project: String, draft: Draft) -> DashboardResult<Watching> {
    match super::ask(Request::Amend { project, draft }).await? {
        Response::Projects(watching) => Ok(watching),
        other => Err(super::unexpected(&other)),
    }
}

/// Stops watching a repository, and reclaims everything it was holding.
///
/// # Errors
///
/// Fails if nothing is watched under that identifier, or if any of its jobs
/// is still working.
#[post("/api/projects/forget")]
pub async fn forget(project: String) -> DashboardResult<Watching> {
    match super::ask(Request::Forget { project }).await? {
        Response::Projects(watching) => Ok(watching),
        other => Err(super::unexpected(&other)),
    }
}

/// The projects screen.
#[component]
pub fn ProjectsView() -> Element {
    let mut reading = use_server_future(projects)?;
    let mut failure = use_signal(|| None::<DashboardError>);
    // What the form is open for, if it is open at all. One signal rather than
    // a flag per purpose — see [`Filling`].
    let mut filling = use_signal(|| None::<Filling>);
    let mut draft = use_signal(Draft::default);

    rsx! {
        div { class: "flex flex-col gap-4",
            match reading.cloned() {
                Some(Ok(watching)) => {
                    // What the project being amended holds now, which is what
                    // decides whether an empty value box means *keep*. Derived
                    // once, so the hint on a box and the control that submits
                    // cannot disagree about the same question — and outside
                    // the markup below, because that is the one place a `let`
                    // cannot go.
                    //
                    // Empty while creating, and that is the true answer rather
                    // than a missing one: a project that does not exist yet
                    // holds nothing.
                    let held: Vec<String> = match filling() {
                        Some(Filling::Amending(ref id)) => watching
                            .projects
                            .iter()
                            .find(|project| &project.id == id)
                            .map(|project| project.variables.clone())
                            .unwrap_or_default(),
                        _ => Vec::new(),
                    };
                    rsx! {
                    Card {
                        title: "Projects",
                        note: "A project is a repository, the agents that work on it, and the \
                               credential those agents need to reach it.",
                        badge: rsx! {
                            Badge { "{watching.projects.len()}" }
                        },
                        aside: rsx! {
                            Button {
                                // A glyph, because the card's title already
                                // says what is being added and repeating it on
                                // the control is the longest thing on the row
                                // saying the least. Named for anyone not
                                // looking at it.
                                class: "px-2.5 text-base leading-none",
                                aria_label: "New project",
                                title: "New project",
                                disabled: watching.available.is_empty(),
                                onclick: {
                                    // The first configured agent as it comes,
                                    // which is what the foreman and the one
                                    // starting kit are seeded with. The kit is
                                    // named and described after the agent, so
                                    // a project with nothing particular to say
                                    // can be saved as it opens — and a kit
                                    // that says more is a row edited rather
                                    // than a row invented.
                                    let first = watching
                                        .shapes
                                        .first()
                                        .map(|shape| {
                                            let fitted = seeded(shape);
                                            let agent = watching
                                                .available
                                                .iter()
                                                .find(|agent| agent.id == shape.agent);
                                            KitDraft {
                                                name: agent
                                                    .map(|agent| agent.name.clone())
                                                    .unwrap_or_default(),
                                                description: agent
                                                    .map(|agent| agent.description.clone())
                                                    .unwrap_or_default(),
                                                fitted,
                                            }
                                        });
                                    move |_| {
                                        // Emptied on the way in rather than on
                                        // the way out, so that a modal
                                        // abandoned half-filled does not
                                        // reopen holding what was abandoned.
                                        draft.set(Draft {
                                            foreman: first
                                                .as_ref()
                                                .map(|kit| kit.fitted.clone())
                                                .unwrap_or_default(),
                                            kits: first.clone().into_iter().collect(),
                                            ..Draft::default()
                                        });
                                        failure.set(None);
                                        filling.set(Some(Filling::Creating));
                                    }
                                },
                                "+"
                            }
                        },
                        if watching.projects.is_empty() {
                            EmptyState {
                                title: "Nothing is being watched yet.",
                                note: if watching.available.is_empty() {
                                    "A project names one agent to think with and at least one its \
                                     jobs run on, so configuring an agent comes first."
                                } else {
                                    "Add one. It needs a repository, the agents that work on it, \
                                     and a credential to reach it with."
                                },
                            }
                        } else {
                            ul { class: "divide-y divide-border",
                                for project in watching.projects.iter().cloned() {
                                    li { key: "{project.id}",
                                        WatchedProject {
                                            project: project.clone(),
                                            available: watching.available.clone(),
                                            // Seeded from what the row already
                                            // holds, so the form opens showing
                                            // what is true rather than blank.
                                            // The credential and the channel
                                            // cannot be among them: neither
                                            // ever reaches a browser.
                                            onedit: {
                                                move |()| {
                                                    draft
                                                        .set(Draft {
                                                            name: project.name.clone(),
                                                            repository: project.repository.clone(),
                                                            foreman: project.foreman.clone(),
                                                            kits: project.kits.clone(),
                                                            credential: String::new(),
                                                            channel: ChannelDraft::default(),
                                                            // Names with empty
                                                            // values: the row
                                                            // says keep, and
                                                            // deleting it says
                                                            // remove.
                                                            variables: project
                                                                .variables
                                                                .iter()
                                                                .map(|name| VariableDraft {
                                                                    name: name.clone(),
                                                                    value: String::new(),
                                                                })
                                                                .collect(),
                                                            brief: project.brief.clone(),
                                                        });
                                                    failure.set(None);
                                                    filling.set(Some(Filling::Amending(project.id.clone())));
                                                }
                                            },
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if let Some(open) = filling() {
                        Modal {
                            title: if open == Filling::Creating { "New project" } else { "Edit project" },
                            onclose: move |()| filling.set(None),
                            actions: rsx! {
                                Button {
                                    // Unavailable until pressing it would
                                    // work, which is this screen's whole
                                    // answer to an incomplete form for now —
                                    // per-field messages are the better
                                    // answer and are not this change.
                                    class: "px-2.5 text-base leading-none",
                                    aria_label: "Save",
                                    title: "Save",
                                    disabled: !draft().is_complete(&open, &held),
                                    onclick: {
                                        let open = open.clone();
                                        move |_| {
                                            let open = open.clone();
                                            async move {
                                                let asked = draft();
                                                // The one place the two callers
                                                // differ: same draft, same
                                                // failure handling, different
                                                // route.
                                                let answered = match open {
                                                    Filling::Creating => {
                                                        create(asked).await
                                                    }
                                                    Filling::Amending(project) => {
                                                        amend(project, asked).await
                                                    }
                                                };
                                                match answered {
                                                    Ok(fresh) => {
                                                        failure.set(None);
                                                        reading.set(Some(Ok(fresh)));
                                                        filling.set(None);
                                                    }
                                                    // Left open, deliberately:
                                                    // closing would throw away
                                                    // what was typed, and what
                                                    // is wrong is almost always
                                                    // in one field of it.
                                                    Err(reason) => failure.set(Some(reason)),
                                                }
                                            }
                                        }
                                    },
                                    "✓"
                                }
                            },
                            // Shown here rather than behind the modal, which is
                            // where it used to be. Anything the screen could
                            // have seen is caught by the control above being
                            // unavailable, so what reaches this is a refusal
                            // only the instance could make.
                            if let Some(reason) = failure() {
                                p { class: "mb-3 text-sm text-failed", "{reason}" }
                            }
                            ProjectForm {
                                draft,
                                available: watching.available,
                                shapes: watching.shapes,
                                filling: open,
                                held,
                            }
                        }
                    }
                }
                },
                Some(Err(reason)) => rsx! {
                    Card { title: "The projects could not be read",
                        p { class: "text-sm text-failed", "{reason}" }
                    }
                },
                None => rsx! {
                    p { class: "text-sm text-muted-foreground", "Reading the projects…" }
                },
            }
        }
    }
}

/// What to show for an agent the browser named by its identifier.
///
/// A function rather than a closure inside the row, so that it can be asserted:
/// mutation testing found the comparison inside it unguarded, which is fair —
/// a project carries identifiers and the row shows names, and nothing else
/// notices if the two stop lining up.
///
/// An identifier this build does not know is shown as it stands rather than
/// hidden. An instance naming an agent that is gone is worth seeing, and
/// `State::check` refuses one anyway.
fn shown_as(available: &[Agent], identifier: &str) -> String {
    available
        .iter()
        .find(|agent| agent.id == identifier)
        .map_or_else(|| identifier.to_owned(), |agent| agent.name.clone())
}

/// One project, as the list shows it.
///
/// It shows and it opens the form; it never edits. What a project *is* stays
/// decided in one place, and a row that edited in place would be a second
/// place, disagreeing about which fields matter and which are required.
/// Changing one is that same form over different initial values, which is what
/// [`ProjectForm`] has always taken and what [`Filling`] now selects between.
///
/// It takes the available agents in order to *render*: a project carries the
/// identifiers a browser sends back, and this is where they become the names a
/// person reads. An identifier this build does not know is shown as it stands
/// rather than hidden — an instance naming an agent that is gone is worth
/// seeing, and `State::check` refuses one anyway.
#[component]
fn WatchedProject(project: Project, available: Vec<Agent>, onedit: EventHandler<()>) -> Element {
    let foreman = shown_as(&available, &project.foreman.agent);
    let offers = project
        .kits
        .iter()
        .map(|kit| kit.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    // The platform's identifiers, because a name costs a scope the manifest
    // does not grant; shown at all because a watched room is where a signal
    // comes from, which is worth seeing at a glance.
    let watching = project.watched.join(", ");
    rsx! {
        // Roomier than the rows on the agents screen, and deliberately: an
        // agent is one line and a project is three, so the same padding reads
        // as cramped here.
        //
        // The first and last shed their outer padding entirely, so the space
        // above the first row and below the last are both the card's own and
        // therefore equal. Anything else makes the top gap the sum of two
        // paddings and the eye reads it as a mistake.
        div { class: "flex flex-col gap-1.5 py-4 first:pt-0 last:pb-0",
            div { class: "flex items-baseline gap-3",
                Link {
                    to: super::Route::ProjectJobsView {
                        project: project.id.clone(),
                    },
                    class: "text-sm font-medium hover:underline",
                    "{project.name}"
                }
                span { class: "truncate font-mono text-xs text-faint-foreground",
                    "{project.repository}"
                }
                span { class: "ml-auto flex shrink-0 items-center gap-2",
                    if project.working > 0 {
                        Badge { tone: BadgeTone::Working, "{project.working} of {project.jobs} working" }
                    } else {
                        Badge { "{project.jobs} job(s)" }
                    }
                    // Offered whatever the project is doing, unlike forgetting
                    // one: amending changes what the next job is given and
                    // cannot reach into a container that already exists.
                    //
                    // A glyph on the same terms as the `+` that adds a project
                    // — the row already says which project this is, so a word
                    // here would be the longest thing on it saying the least.
                    // Named for anyone not looking at it, and named with the
                    // project, because a screen of these reads out as a column
                    // of identical "Edit"s otherwise.
                    //
                    // U+270E and deliberately not U+270F, which is the pencil
                    // most editors offer: that one has an emoji presentation
                    // and most platforms take it, so it would arrive in colour
                    // beside the monochrome `×`, `+` and `✓` this dashboard
                    // already uses. The glyph vocabulary here is text, and one
                    // emoji in it looks like a mistake rather than a choice.
                    Button {
                        // Secondary, because this sits beside a badge on every
                        // row: the enum's own word for it is "an ordinary
                        // action sitting beside others", and a solid accent
                        // repeated down the list would out-shout the one
                        // primary action the screen has.
                        variant: ButtonVariant::Secondary,
                        class: "px-2 py-1 text-sm leading-none",
                        aria_label: "Edit {project.name}",
                        title: "Edit",
                        onclick: move |_| onedit.call(()),
                        "✎"
                    }
                }
            }
            p { class: "text-xs text-muted-foreground",
                "thinks with {foreman} · offers {offers}"
                if project.platforms.is_empty() {
                    " · no credential"
                }
                // Absence only, in the same way the credential above is. A
                // bound channel needs no announcement; one that is missing
                // changes what the project can be asked to do.
                if project.channels.is_empty() {
                    " · no channel"
                }
                // Presence, unlike the two above, and the asymmetry is the
                // point: a project with no variables is the ordinary case and
                // says nothing, while one carrying third-party credentials is
                // worth seeing at a glance. Counted rather than named, because
                // the row is already three facts long.
                if !project.variables.is_empty() {
                    " · {project.variables.len()} variable(s)"
                }
                if !project.watched.is_empty() {
                    " · watching {watching}"
                }
            }
        }
    }
}

/// The form that describes a project.
///
/// **Controlled**: the caller owns the draft and this writes into it. That is
/// what lets the control which submits live outside the form — in the modal's
/// header, beside the way out — and it is what an edit will need too, since
/// editing is this form over different initial values with a different handler.
///
/// It renders no submit of its own. A form that both collected and committed
/// would have to be told where its button goes, which is the caller's business
/// and not its own.
#[component]
fn ProjectForm(
    draft: Signal<Draft>,
    available: Vec<Agent>,
    shapes: Vec<Shape>,
    filling: Filling,
    held: Vec<String>,
) -> Element {
    let mut draft = draft;
    let creating = filling.creating();
    // What a kit added to the form starts on: the first agent as it comes.
    let starting = shapes.first().map(seeded).unwrap_or_default();

    rsx! {
        div { class: "flex flex-col gap-3",
            Field { label: "Name",
                input {
                    class: FIELD,
                    placeholder: "what to call it",
                    value: "{draft().name}",
                    oninput: move |event| draft.with_mut(|draft| draft.name = event.value()),
                }
            }
            Field { label: "Repository",
                input {
                    class: FIELD,
                    placeholder: "https://github.com/…",
                    value: "{draft().repository}",
                    oninput: move |event| draft.with_mut(|draft| draft.repository = event.value()),
                }
            }
            Field { label: "Thinks with",
                FittedEditor {
                    fitted: draft().foreman,
                    shapes: shapes.clone(),
                    available: available.clone(),
                    onchange: move |fitted| draft.with_mut(|draft| draft.foreman = fitted),
                }
            }
            p { class: "text-xs text-faint-foreground",
                "A change here lands when the foreman next picks up a message, never in the \
                 middle of one. Changing the agent starts its memory over; changing only the \
                 model or the effort keeps it."
            }
            Field { label: "Runs jobs on",
                div { class: "flex flex-col gap-2",
                    for (position, row) in draft().kits.iter().enumerate() {
                        div { key: "{position}", class: "flex flex-col gap-2 rounded-md border border-border p-2",
                            div { class: "flex items-center gap-2",
                                input {
                                    class: FIELD,
                                    placeholder: "a name, e.g. quick",
                                    value: "{row.name}",
                                    oninput: move |event| {
                                        draft
                                            .with_mut(|draft| {
                                                if let Some(row) = draft.kits.get_mut(position) {
                                                    row.name = event.value();
                                                }
                                            });
                                    },
                                }
                                // Removing the row is how a kit is taken away.
                                Button {
                                    variant: ButtonVariant::Secondary,
                                    class: "shrink-0 px-2 py-1 text-sm leading-none",
                                    aria_label: "Remove kit {position + 1}",
                                    title: "Remove",
                                    onclick: move |_| {
                                        draft.with_mut(|draft| { draft.kits.remove(position); });
                                    },
                                    "×"
                                }
                            }
                            input {
                                class: FIELD,
                                placeholder: "what this project wants it for, e.g. small fixes and questions",
                                value: "{row.description}",
                                oninput: move |event| {
                                    draft
                                        .with_mut(|draft| {
                                            if let Some(row) = draft.kits.get_mut(position) {
                                                row.description = event.value();
                                            }
                                        });
                                },
                            }
                            FittedEditor {
                                fitted: row.fitted.clone(),
                                shapes: shapes.clone(),
                                available: available.clone(),
                                onchange: move |fitted| {
                                    draft
                                        .with_mut(|draft| {
                                            if let Some(row) = draft.kits.get_mut(position) {
                                                row.fitted = fitted;
                                            }
                                        });
                                },
                            }
                        }
                    }
                    Button {
                        variant: ButtonVariant::Secondary,
                        class: "self-start px-2.5 py-1 text-sm leading-none",
                        aria_label: "Add a kit",
                        title: "Add a kit",
                        onclick: {
                            move |_| {
                                let fitted = starting.clone();
                                draft.with_mut(|draft| {
                                    draft.kits.push(KitDraft {
                                        fitted,
                                        ..KitDraft::default()
                                    });
                                });
                            }
                        },
                        "+"
                    }
                }
            }
            p { class: "text-xs text-faint-foreground",
                "A kit is an agent set a particular way. The foreman picks one per job by what \
                 you say it is for, so say what each is for — a cheap one for small fixes and \
                 questions, a strong one for work that touches many files."
            }
            Field { label: "Brief (optional)",
                textarea {
                    class: "{FIELD} min-h-24",
                    placeholder: "standing instructions for the foreman: which alerts to ignore, \
                                  what a filed issue deserves, which account its jobs act as…",
                    value: "{draft().brief}",
                    oninput: move |event| draft.with_mut(|draft| draft.brief = event.value()),
                }
            }
            p { class: "text-xs text-faint-foreground",
                "Said to the foreman every time it is asked anything or a watched room hears \
                 another app, in your words, so this is where policy lives. It is followed by \
                 judgement rather than enforced, and every line costs tokens on every turn. \
                 Ask the foreman in a channel to watch it; what it watches is listed on the row."
            }
            Field { label: if creating { "GitHub credential" } else { "GitHub credential (optional)" },
                input {
                    r#type: "password",
                    class: FIELD,
                    // The one place this form tells the operator what an empty
                    // box means, because it is the one place where empty is not
                    // the same as absent. There is nothing to prefill it with:
                    // no credential ever reaches the browser.
                    placeholder: if creating {
                        "a token scoped to this repository"
                    } else {
                        "leave empty to keep the current one"
                    },
                    value: "{draft().credential}",
                    oninput: move |event| draft.with_mut(|draft| draft.credential = event.value()),
                }
            }
            p { class: "text-xs text-faint-foreground",
                "Scoped to this repository, with contents and pull requests write. A token that \
                 reaches more than this project is a token every job on it could misuse."
            }
            // Only when creating. A binding's credentials never reach the
            // browser, so there is nothing to show here for a project that has
            // one — and an empty box that meant *unbind* would disconnect a
            // project every time somebody corrected its name.
            if creating {
            Field { label: "Slack bot token",
                input {
                    r#type: "password",
                    class: FIELD,
                    placeholder: "xoxb-…, what speaks",
                    value: "{draft().channel.credential}",
                    oninput: move |event| {
                        draft.with_mut(|draft| draft.channel.credential = event.value());
                    },
                }
            }
            Field { label: "Slack app-level token",
                input {
                    r#type: "password",
                    class: FIELD,
                    placeholder: "xapp-…, what listens",
                    value: "{draft().channel.listen_credential}",
                    oninput: move |event| {
                        draft.with_mut(|draft| draft.channel.listen_credential = event.value());
                    },
                }
            }
            p { class: "text-xs text-faint-foreground",
                "Every project talks on Slack, through an app of its own: it hears any channel \
                 it is invited to, and every job gets a channel of its own. Both tokens are \
                 required, because a job that asks needs somebody able to answer."
            }
            }
            Field { label: "Variables (optional)",
                div { class: "flex flex-col gap-2",
                    for (position, row) in draft().variables.iter().enumerate() {
                        div { key: "{position}", class: "flex items-center gap-2",
                            input {
                                class: "{FIELD} font-mono",
                                placeholder: "STRIPE_API_KEY",
                                value: "{row.name}",
                                oninput: move |event| {
                                    draft
                                        .with_mut(|draft| {
                                            if let Some(row) = draft.variables.get_mut(position) {
                                                row.name = event.value();
                                            }
                                        });
                                },
                            }
                            input {
                                r#type: "password",
                                class: FIELD,
                                // Per row rather than per form, because *this
                                // row* is what decides it: a box says "keep"
                                // only where there is something to keep, which
                                // is a name the project already holds. A row
                                // just added holds nothing, and neither does
                                // one whose name has been typed over — and
                                // both change the moment the name does, which
                                // is the behaviour an operator expects from a
                                // hint about the box beside it.
                                placeholder: if held.iter().any(|had| had == row.name.trim()) {
                                    "leave empty to keep"
                                } else {
                                    "its value"
                                },
                                value: "{row.value}",
                                oninput: move |event| {
                                    draft
                                        .with_mut(|draft| {
                                            if let Some(row) = draft.variables.get_mut(position) {
                                                row.value = event.value();
                                            }
                                        });
                                },
                            }
                            // Removing the row is how a variable is taken
                            // away: an empty value already means keep, so
                            // absence is the only thing left to mean drop.
                            Button {
                                variant: ButtonVariant::Secondary,
                                class: "shrink-0 px-2 py-1 text-sm leading-none",
                                aria_label: "Remove variable {position + 1}",
                                title: "Remove",
                                onclick: move |_| {
                                    draft.with_mut(|draft| { draft.variables.remove(position); });
                                },
                                "×"
                            }
                        }
                    }
                    Button {
                        variant: ButtonVariant::Secondary,
                        class: "self-start px-2.5 py-1 text-sm leading-none",
                        aria_label: "Add a variable",
                        title: "Add a variable",
                        onclick: move |_| {
                            draft.with_mut(|draft| draft.variables.push(VariableDraft::default()));
                        },
                        "+"
                    }
                }
            }
            p { class: "text-xs text-faint-foreground",
                "Set in every container this project's jobs run in, and named to the agent so it \
                 knows they are there. stageman never reads one, so what they mean is the \
                 repository's business. Removing a row takes the variable away; leaving its value \
                 empty keeps the one already stored."
            }
        }
    }
}

/// The three selects that set an agent: which one, which model, and how hard
/// it thinks where the model allows a choice.
///
/// **Controlled**, like the form around it: it emits the whole [`Fitted`] on
/// every change rather than writing anywhere, so that the parent decides which
/// row it lands in. Moving to another agent starts from that agent's defaults;
/// moving to another model keeps the effort where the new one takes it and
/// clears it where it does not — see [`with_agent`] and [`with_model`], which
/// are pure so that both rules can be tested without a browser.
///
/// The effort select appears only where the model takes one. A select for a
/// setting the model does not have would offer a choice the far side refuses.
#[component]
fn FittedEditor(
    fitted: Fitted,
    shapes: Vec<Shape>,
    available: Vec<Agent>,
    onchange: EventHandler<Fitted>,
) -> Element {
    let shape = shape_for(&shapes, &fitted.agent).cloned();
    let with_effort = shape
        .as_ref()
        .is_some_and(|shape| takes_effort(shape, &fitted.model));

    rsx! {
        div { class: "flex flex-wrap gap-2",
            select {
                class: "{FIELD} basis-40 grow",
                value: "{fitted.agent}",
                onchange: move |event: Event<FormData>| {
                    onchange.call(with_agent(&shapes, &event.value()));
                },
                for agent in available.iter() {
                    option { key: "{agent.id}", value: "{agent.id}", "{agent.name}" }
                }
            }
            if let Some(shape) = shape {
                select {
                    class: "{FIELD} basis-40 grow",
                    value: "{fitted.model}",
                    onchange: {
                        let fitted = fitted.clone();
                        move |event: Event<FormData>| {
                            onchange.call(with_model(&fitted, &shape, &event.value()));
                        }
                    },
                    for model in shape.models.iter() {
                        option { key: "{model.id}", value: "{model.id}", "{model.name}" }
                    }
                }
                if with_effort {
                    select {
                        class: "{FIELD} basis-40 grow",
                        value: "{fitted.effort}",
                        onchange: {
                            let fitted = fitted.clone();
                            move |event: Event<FormData>| {
                                onchange.call(Fitted {
                                    effort: event.value(),
                                    ..fitted.clone()
                                });
                            }
                        },
                        for effort in shape.efforts.iter() {
                            option { key: "{effort.id}", value: "{effort.id}", "{effort.name}" }
                        }
                    }
                }
            }
        }
    }
}

/// What every input on this screen looks like.
///
/// A constant rather than a component, because the thing being shared is the
/// appearance of a box and not its behaviour — a `select` and an `input`
/// differ in everything except how they should look.
const FIELD: &str = "w-full rounded-md border border-border bg-surface px-2 py-1.5 \
                     text-sm placeholder:text-faint-foreground focus-visible:outline-none \
                     focus-visible:ring-2 focus-visible:ring-primary";

/// A labelled row in the form.
#[component]
fn Field(label: String, children: Element) -> Element {
    rsx! {
        label { class: "flex flex-col gap-1",
            span { class: "text-xs font-medium text-muted-foreground", "{label}" }
            {children}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::agents_view::Agent;
    use super::{
        Choice, Fitted, ModelChoice, Shape, seeded, shape_for, shown_as, takes_effort, with_agent,
        with_model,
    };

    /// The agent's defaults, as a browser holds them.
    fn as_it_comes() -> Fitted {
        Fitted {
            agent: "claude".to_owned(),
            model: "default".to_owned(),
            effort: "default".to_owned(),
        }
    }

    /// The shape the server would send for Claude, as the form sees it.
    fn claude() -> Shape {
        let model = |id: &str, has_effort: bool| ModelChoice {
            id: id.to_owned(),
            name: id.to_owned(),
            has_effort,
        };
        let effort = |id: &str| Choice {
            id: id.to_owned(),
            name: id.to_owned(),
        };
        Shape {
            agent: "claude".to_owned(),
            models: vec![
                model("default", true),
                model("sonnet", true),
                model("opus", true),
                model("haiku", false),
            ],
            efforts: vec![effort("default"), effort("low"), effort("high")],
        }
    }

    /// A project carries identifiers and a row shows names, so something has
    /// to map one to the other — and nothing else would notice if it stopped.
    ///
    /// Found by mutation testing: inverting the comparison inside the row's
    /// lookup broke nothing any test could see, which is exactly the shape of
    /// a screen that renders confidently and wrongly.
    #[test]
    fn an_agent_is_shown_by_its_name_and_not_the_identifier_it_arrived_as() {
        let available = vec![Agent {
            id: "claude".to_owned(),
            name: "Claude".to_owned(),
            description: "does the work".to_owned(),
            configured: true,
            used_by: Vec::new(),
        }];

        assert_eq!(shown_as(&available, "claude"), "Claude");
    }

    /// An agent this build does not know is shown as it stands.
    ///
    /// The other half, and the reason the lookup falls back rather than
    /// hiding: an instance naming an agent that is gone is worth seeing.
    #[test]
    fn an_agent_this_build_does_not_know_is_shown_as_it_arrived() {
        assert_eq!(shown_as(&[], "something-else"), "something-else");
    }

    /// Moving between models keeps, clears or seeds the effort as the new
    /// model demands, so the form never holds a pair the far side refuses.
    #[test]
    fn changing_the_model_keeps_the_effort_only_where_the_new_model_takes_one() {
        let shape = claude();
        let on_opus = with_model(&as_it_comes(), &shape, "opus");
        assert_eq!(on_opus.model, "opus");
        assert_eq!(on_opus.effort, "default", "kept, since opus takes one");

        let on_haiku = with_model(&on_opus, &shape, "haiku");
        assert_eq!(on_haiku.model, "haiku");
        assert_eq!(on_haiku.effort, "", "cleared, since haiku takes none");

        let back = with_model(&on_haiku, &shape, "sonnet");
        assert_eq!(
            back.effort, "default",
            "seeded with the first, since there was none"
        );

        let chosen = Fitted {
            effort: "high".to_owned(),
            ..as_it_comes()
        };
        assert_eq!(
            with_model(&chosen, &shape, "sonnet").effort,
            "high",
            "a chosen effort survives a change of model"
        );
    }

    /// The two comparisons the form turns on, asserted in both directions.
    ///
    /// Mutation testing inverted each of these inside the component and no
    /// test noticed, which is why they are functions now.
    #[test]
    fn a_shape_is_found_by_its_agent_and_says_which_models_take_an_effort() {
        let shapes = vec![claude()];
        assert_eq!(shape_for(&shapes, "claude"), Some(&claude()));
        assert_eq!(shape_for(&shapes, "gpt"), None);
        assert_eq!(shape_for(&[], "claude"), None);

        assert!(takes_effort(&claude(), "opus"));
        assert!(takes_effort(&claude(), "default"));
        assert!(!takes_effort(&claude(), "haiku"), "the one model with none");
        assert!(
            !takes_effort(&claude(), "gpt-5"),
            "a model the shape does not list takes nothing"
        );
    }

    /// Moving to another agent starts from that agent's defaults, and an
    /// agent no shape describes is carried as itself for the far side to
    /// refuse by name.
    #[test]
    fn changing_the_agent_starts_from_its_defaults() {
        let shapes = vec![claude()];
        assert_eq!(with_agent(&shapes, "claude"), as_it_comes());
        assert_eq!(seeded(&claude()), as_it_comes());
        assert_eq!(
            with_agent(&shapes, "gpt"),
            Fitted {
                agent: "gpt".to_owned(),
                ..Fitted::default()
            }
        );

        let mut effortless = claude();
        effortless.models.rotate_left(3);
        assert_eq!(
            seeded(&effortless).effort,
            "",
            "an agent whose first model takes no effort is seeded with none"
        );
    }
}

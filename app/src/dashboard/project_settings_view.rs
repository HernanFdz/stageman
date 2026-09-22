//! A project's settings, and a new project, which is the same page with
//! nothing filled in.
//!
//! A page rather than a panel over one, per
//! `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`: the
//! form has as many fields as a page, it has sections, and it has an address
//! to come back to. Both pages read what the projects screen reads, because
//! creating needs the agents that may be named and the shape of each, and
//! amending needs those and the project as it is.
//!
//! **Validity is asked, not restated.** What makes an instance valid is
//! `State::check` and nothing else; this page builds the state it would
//! produce, asks, and reports the answer beside the box it concerns where it
//! can. What it does first is refuse to ask badly: the wire knows which boxes
//! are empty, and the page says so beside each once a save has been tried.
//!
//! Every closed set here is a row of options rather than a dropdown, and
//! every longer text a box that grows, per `docs/conventions.md` §3.

use dioxus::prelude::*;

use super::agents_view::Agent;
use super::error::DashboardError;
use super::live::Live;
use super::projects_view::{amend, create, forget, projects};
use crate::ui::{
    BESIDE, Button, ButtonVariant, Card, FIELD, Field, Icon, Modal, Segmented, Skeleton, TextArea,
    Tooltip,
};

pub use stageman_wire::{
    ChannelDraft, Draft, Filling, Fitted, KitDraft, Part, Shape, VariableDraft, Watching,
};

/// An agent as it comes: the shape's first model, and its first effort where
/// that model takes one.
///
/// What a new kit starts on, and what a fitted agent moves to when its agent
/// changes — nothing carries over between agents, because a model is one
/// agent's and not another's.
pub(super) fn seeded(shape: &Shape) -> Fitted {
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
/// A function rather than a `find` at each of its call sites, so that the
/// comparison it turns on is tested once — mutation testing inverted it inside
/// the component, where nothing could notice.
pub(super) fn shape_for<'a>(shapes: &'a [Shape], agent: &str) -> Option<&'a Shape> {
    shapes.iter().find(|shape| shape.agent == agent)
}

/// Whether a model takes an effort, as its agent's shape says.
///
/// False for a model the shape does not list at all, which the form cannot
/// produce and a request written by hand can: the far side refuses it by name,
/// and offering an effort for it here would be offering a second thing to
/// refuse.
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

/// The draft a page starts from: what the project holds, or — for one that
/// does not exist yet — the first configured agent as it comes, for the
/// foreman and for one starting kit named and described after the agent, so
/// that a project with nothing particular to say can be saved as it opens.
///
/// The credential and the channel cannot be seeded: neither ever reaches a
/// browser. A variable is seeded as its name with an empty value, which the
/// wire reads as *keep*.
fn starting(watching: &Watching, filling: &Filling) -> Draft {
    match filling {
        Filling::Creating => {
            let first = watching.shapes.first().map(|shape| {
                let fitted = seeded(shape);
                let agent = watching
                    .available
                    .iter()
                    .find(|agent| agent.id == shape.agent);
                KitDraft {
                    name: agent.map(|agent| agent.name.clone()).unwrap_or_default(),
                    description: agent
                        .map(|agent| agent.description.clone())
                        .unwrap_or_default(),
                    fitted,
                }
            });
            Draft {
                foreman: first
                    .as_ref()
                    .map(|kit| kit.fitted.clone())
                    .unwrap_or_default(),
                kits: first.into_iter().collect(),
                ..Draft::default()
            }
        }
        Filling::Amending(id) => watching
            .projects
            .iter()
            .find(|project| &project.id == id)
            .map(|project| Draft {
                name: project.name.clone(),
                repository: project.repository.clone(),
                foreman: project.foreman.clone(),
                kits: project.kits.clone(),
                credential: String::new(),
                channel: ChannelDraft::default(),
                variables: project
                    .variables
                    .iter()
                    .map(|name| VariableDraft {
                        name: name.clone(),
                        value: String::new(),
                    })
                    .collect(),
                brief: project.brief.clone(),
            })
            .unwrap_or_default(),
    }
}

/// The page for a project that does not exist yet.
#[component]
pub fn ProjectNewView() -> Element {
    let live = use_context::<Live>();
    let reading = use_server_future(move || {
        let _ = live.follow();
        projects()
    })?;

    rsx! {
        match reading.cloned() {
            Some(Ok(watching)) => rsx! { Editing { watching, filling: Filling::Creating } },
            Some(Err(reason)) => rsx! {
                Card { title: "The projects could not be read",
                    p { class: "text-sm text-failed", "{reason}" }
                }
            },
            None => rsx! { Skeleton {} },
        }
    }
}

/// The page for a project that exists.
#[component]
pub fn ProjectSettingsView(project: String) -> Element {
    let live = use_context::<Live>();
    let reading = use_server_future(use_reactive!(|project| {
        let _ = live.follow();
        let _ = project;
        projects()
    }))?;

    rsx! {
        match reading.cloned() {
            Some(Ok(watching)) => {
                if watching.projects.iter().any(|known| known.id == project) {
                    rsx! { Editing { watching, filling: Filling::Amending(project) } }
                } else {
                    rsx! {
                        Card { title: "No such project",
                            p { class: "text-sm text-muted-foreground",
                                "Nothing is watched under this identifier. It may have been forgotten."
                            }
                        }
                    }
                }
            }
            Some(Err(reason)) => rsx! {
                Card { title: "The project could not be read",
                    p { class: "text-sm text-failed", "{reason}" }
                }
            },
            None => rsx! { Skeleton {} },
        }
    }
}

/// The form, in sections, with what commits it at the top.
///
/// **Controlled by this page**: the draft is one signal and every section
/// writes into it, so that the control which saves can read the whole. A
/// problem is shown beside its box once a save has been tried, and not
/// before: a form that opens red is a form that scolds before anybody has
/// typed. What the instance refuses is shown beside its box too, where the
/// refusal says which, and at the top where it does not.
#[component]
fn Editing(watching: Watching, filling: Filling) -> Element {
    let seed = starting(&watching, &filling);
    let mut draft = use_signal(move || seed);
    let mut tried = use_signal(|| false);
    let mut refused = use_signal(|| None::<DashboardError>);
    let mut forgetting = use_signal(|| false);
    let creating = filling.creating();

    // What the project holds now, which decides whether an empty value box
    // means *keep*. Empty while creating, which is the true answer: a
    // project that does not exist yet holds nothing.
    let held: Vec<String> = match &filling {
        Filling::Amending(id) => watching
            .projects
            .iter()
            .find(|project| &project.id == id)
            .map(|project| project.variables.clone())
            .unwrap_or_default(),
        Filling::Creating => Vec::new(),
    };
    let name = match &filling {
        Filling::Amending(id) => watching
            .projects
            .iter()
            .find(|project| &project.id == id)
            .map(|project| project.name.clone())
            .unwrap_or_default(),
        Filling::Creating => String::new(),
    };

    let problems = if tried() {
        draft().problems(&filling, &held)
    } else {
        Vec::new()
    };
    let refusal = refused();
    let pointed = refusal.as_ref().and_then(|reason| match reason {
        DashboardError::Refused(refusal) => refusal.part(),
        DashboardError::NoInstance | DashboardError::Failed => None,
    });
    // What to say beside a part: the first thing wrong with it, or the
    // instance's refusal where that points here.
    let saying = move |part: Part| -> Option<String> {
        problems
            .iter()
            .find(|problem| problem.part == part)
            .map(|problem| problem.why.clone())
            .or_else(|| {
                (pointed == Some(part))
                    .then(|| refusal.as_ref().map(ToString::to_string))
                    .flatten()
            })
    };
    let unplaced = refused().filter(|_| pointed.is_none());

    let shapes = watching.shapes.clone();
    let available = watching.available;
    let starting_kit = shapes.first().map(seeded).unwrap_or_default();
    let back = match &filling {
        Filling::Creating => super::Route::ProjectsView {},
        Filling::Amending(id) => super::Route::ProjectJobsView {
            project: id.clone(),
        },
    };

    let save = {
        let filling = filling.clone();
        let held = held.clone();
        move |_| {
            tried.set(true);
            let asked = draft();
            if !asked.problems(&filling, &held).is_empty() {
                return;
            }
            let filling = filling.clone();
            let navigator = navigator();
            spawn(async move {
                let answered = match &filling {
                    Filling::Creating => create(asked).await,
                    Filling::Amending(project) => amend(project.clone(), asked).await,
                };
                match answered {
                    Ok(_) => {
                        refused.set(None);
                        // Where the project is: its own page once it has
                        // one, and the list when it has just been made,
                        // since nothing names a project uniquely but the
                        // identifier the instance minted.
                        match filling {
                            Filling::Creating => {
                                navigator.push(super::Route::ProjectsView {});
                            }
                            Filling::Amending(project) => {
                                navigator.push(super::Route::ProjectJobsView { project });
                            }
                        }
                    }
                    Err(reason) => refused.set(Some(reason)),
                }
            });
        }
    };

    rsx! {
        div { class: "flex flex-col gap-4",
            div { class: "flex items-baseline gap-3",
                h1 { class: "text-base font-semibold",
                    if creating { "New project" } else { "{name}" }
                }
                if !creating {
                    span { class: "text-sm text-muted-foreground", "settings" }
                }
                span { class: "ml-auto flex items-center gap-2",
                    Link {
                        to: back,
                        class: ButtonVariant::Secondary.styled(""),
                        "Cancel"
                    }
                    Button { onclick: save, if creating { "Create" } else { "Save" } }
                }
            }
            if let Some(reason) = unplaced {
                p { role: "alert", class: "text-sm text-failed", "{reason}" }
            }

            Card { title: "About",
                div { class: "flex flex-col gap-3",
                    Field { label: "Name", problem: saying(Part::Name),
                        input {
                            class: FIELD,
                            placeholder: "Closed Loop",
                            value: "{draft().name}",
                            oninput: move |event| draft.with_mut(|draft| draft.name = event.value()),
                        }
                    }
                    Field {
                        label: "Repository",
                        note: "The address on GitHub its jobs work on.",
                        problem: saying(Part::Repository),
                        input {
                            class: "{FIELD} font-mono",
                            placeholder: "https://github.com/owner/repository",
                            value: "{draft().repository}",
                            oninput: move |event| draft.with_mut(|draft| draft.repository = event.value()),
                        }
                    }
                }
            }

            Card {
                title: "Agents",
                note: "One to think with, and the kits its jobs may run on.",
                div { class: "flex flex-col gap-4",
                    Field {
                        label: "Thinks with",
                        note: "The foreman's own agent. A change lands at its next turn.",
                        info: "A change here lands when the foreman next picks up a message, \
                               never in the middle of one. Changing the agent starts its memory \
                               over; changing only the model or the effort keeps it.",
                        problem: saying(Part::Foreman),
                        FittedEditor {
                            fitted: draft().foreman,
                            shapes: shapes.clone(),
                            available: available.clone(),
                            onchange: move |fitted| draft.with_mut(|draft| draft.foreman = fitted),
                        }
                    }
                    Field {
                        label: "Runs jobs on",
                        note: "A kit is an agent set a particular way. The foreman picks one per job by what you say it is for.",
                        info: "Say what each kit is for — a cheap one for small fixes and \
                               questions, a strong one for work that touches many files. The \
                               foreman reads these lines when it chooses, so they are the \
                               most useful thing on this page to write well.",
                        problem: saying(Part::Kits),
                        aside: rsx! {
                            Tooltip { text: "Add a kit",
                                Button {
                                    variant: ButtonVariant::Secondary,
                                    class: BESIDE,
                                    aria_label: "Add a kit",
                                    onclick: move |_| {
                                        let fitted = starting_kit.clone();
                                        draft.with_mut(|draft| {
                                            draft.kits.push(KitDraft {
                                                fitted,
                                                ..KitDraft::default()
                                            });
                                        });
                                    },
                                    {Icon::Add.draw(16)}
                                }
                            }
                        },
                        div { class: "flex flex-col divide-y divide-border",
                            for (position, row) in draft().kits.iter().enumerate() {
                                Kit {
                                    key: "{position}",
                                    position,
                                    row: row.clone(),
                                    shapes: shapes.clone(),
                                    available: available.clone(),
                                    problem: saying(Part::Kit(position)),
                                    onchange: move |changed: KitDraft| {
                                        draft.with_mut(|draft| {
                                            if let Some(row) = draft.kits.get_mut(position) {
                                                *row = changed;
                                            }
                                        });
                                    },
                                    onremove: move |()| {
                                        draft.with_mut(|draft| {
                                            if position < draft.kits.len() {
                                                draft.kits.remove(position);
                                            }
                                        });
                                    },
                                }
                            }
                        }
                    }
                }
            }

            Card { title: "Brief",
                Field {
                    label: "Brief",
                    note: "Standing instructions for the foreman, said to it on every turn.",
                    info: "Said to the foreman every time it is asked anything or a watched room \
                           hears another app, in your words, so this is where policy lives: \
                           which alerts to ignore, what a filed issue deserves, which account its \
                           jobs act as. It is followed by judgement rather than enforced, and every \
                           line costs tokens on every turn. Ask the foreman in a channel to watch \
                           a room; what it watches is listed on the project's row.",
                    TextArea {
                        class: "min-h-24",
                        placeholder: "Ignore alerts below error. A filed issue gets a job. Act as the release account.",
                        value: draft().brief,
                        oninput: move |event: FormEvent| draft.with_mut(|draft| draft.brief = event.value()),
                    }
                }
            }

            Card { title: "GitHub",
                Field {
                    label: "Token",
                    note: if creating {
                        "A fine-grained token scoped to this repository, with contents and pull requests write."
                    } else {
                        "Leave empty to keep the current one; a new one replaces it."
                    },
                    info: "Every job on this project holds this token, so a token that reaches \
                           more than this repository is a token every job could misuse. Scope it \
                           to the one repository, with contents and pull requests write and \
                           nothing else.",
                    problem: saying(Part::Credential),
                    input {
                        r#type: "password",
                        class: "{FIELD} font-mono",
                        placeholder: "github_pat_…",
                        value: "{draft().credential}",
                        oninput: move |event| draft.with_mut(|draft| draft.credential = event.value()),
                    }
                }
            }

            // Only when creating. A binding's credentials never reach the
            // browser, so there is nothing to show for a project that has
            // one — and an empty box that meant *unbind* would disconnect a
            // project every time somebody corrected its name.
            if creating {
                Card {
                    title: "Slack",
                    note: "Every project talks on Slack, through an app of its own.",
                    div { class: "flex flex-col gap-3",
                        Field {
                            label: "Bot token",
                            note: "What speaks. Starts with xoxb.",
                            problem: saying(Part::Channel),
                            input {
                                r#type: "password",
                                class: "{FIELD} font-mono",
                                placeholder: "xoxb-…",
                                value: "{draft().channel.credential}",
                                oninput: move |event| {
                                    draft.with_mut(|draft| draft.channel.credential = event.value());
                                },
                            }
                        }
                        Field {
                            label: "App-level token",
                            note: "What listens. Starts with xapp, with the connections:write scope.",
                            info: "The app hears every channel it is invited to, and every job gets \
                                   a channel of its own. Both tokens are required, because a job \
                                   that asks needs somebody able to answer. The manifest to create \
                                   the app from is in the README.",
                            input {
                                r#type: "password",
                                class: "{FIELD} font-mono",
                                placeholder: "xapp-…",
                                value: "{draft().channel.listen_credential}",
                                oninput: move |event| {
                                    draft.with_mut(|draft| draft.channel.listen_credential = event.value());
                                },
                            }
                        }
                    }
                }
            }

            Card {
                title: "Variables",
                note: "Set in every container this project's jobs run in. stageman never reads one.",
                Field {
                    label: "Variables",
                    note: "Names and values, as an environment carries them. Removing a row takes the variable away.",
                    info: "Told to the agent by name so it knows they are there, and never read \
                           here: what they mean is the repository's business. Leaving a value \
                           empty keeps the one already stored.",
                    problem: saying(Part::Variables),
                    aside: rsx! {
                        Tooltip { text: "Add a variable",
                            Button {
                                variant: ButtonVariant::Secondary,
                                class: BESIDE,
                                aria_label: "Add a variable",
                                onclick: move |_| {
                                    draft.with_mut(|draft| draft.variables.push(VariableDraft::default()));
                                },
                                {Icon::Add.draw(16)}
                            }
                        }
                    },
                    div { class: "flex flex-col gap-2",
                        for (position, row) in draft().variables.iter().enumerate() {
                            Variable {
                                key: "{position}",
                                position,
                                row: row.clone(),
                                kept: held.iter().any(|had| had == row.name.trim()),
                                problem: saying(Part::Variable(position)),
                                onchange: move |changed: VariableDraft| {
                                    draft.with_mut(|draft| {
                                        if let Some(row) = draft.variables.get_mut(position) {
                                            *row = changed;
                                        }
                                    });
                                },
                                onremove: move |()| {
                                    draft.with_mut(|draft| {
                                        if position < draft.variables.len() {
                                            draft.variables.remove(position);
                                        }
                                    });
                                },
                            }
                        }
                    }
                }
            }

            if let Filling::Amending(id) = filling {
                Card {
                    title: "Forget this project",
                    note: "Stops watching the repository and removes every job's container. The repository itself is untouched.",
                    Button {
                        variant: ButtonVariant::Danger,
                        onclick: move |_| forgetting.set(true),
                        "Forget…"
                    }
                }
                if forgetting() {
                    Modal {
                        title: "Forget {name}?",
                        onclose: move |()| forgetting.set(false),
                        actions: rsx! {
                            Button {
                                variant: ButtonVariant::Danger,
                                onclick: move |_| {
                                    {
                                        let id = id.clone();
                                        let navigator = navigator();
                                        spawn(async move {
                                            match forget(id).await {
                                                Ok(_) => {
                                                    navigator.push(super::Route::ProjectsView {});
                                                }
                                                Err(reason) => {
                                                    forgetting.set(false);
                                                    refused.set(Some(reason));
                                                }
                                            }
                                        });
                                    }
                                },
                                "Forget"
                            }
                        },
                        p { class: "text-sm text-muted-foreground",
                            "Every job's container and session go with it. A job still working \
                             stops this, so stop it first."
                        }
                    }
                }
            }
        }
    }
}

/// One kit's row: its name, what it is for, and how its agent is set.
#[component]
fn Kit(
    position: usize,
    row: KitDraft,
    shapes: Vec<Shape>,
    available: Vec<Agent>,
    problem: Option<String>,
    onchange: EventHandler<KitDraft>,
    onremove: EventHandler<()>,
) -> Element {
    rsx! {
        // A row parted from the next by a hairline rather than a box within
        // the box, so that its remove control stands on the same edge as
        // every other control in the section — `docs/conventions.md` §3.
        div { class: "flex flex-col gap-2 py-3 first:pt-0 last:pb-0",
            div { class: "flex items-center gap-2",
                input {
                    class: FIELD,
                    placeholder: "a name, e.g. quick",
                    aria_label: "Name of kit {position + 1}",
                    value: "{row.name}",
                    // Moved rather than cloned: the last thing on the row
                    // that wants it, in the order the macro evaluates.
                    oninput: move |event| onchange.call(KitDraft { name: event.value(), ..row.clone() }),
                }
                // Removing the row is how a kit is taken away.
                Tooltip { text: "Remove",
                    Button {
                        variant: ButtonVariant::Secondary,
                        class: BESIDE,
                        aria_label: "Remove kit {position + 1}",
                        onclick: move |_| onremove.call(()),
                        {Icon::Remove.draw(16)}
                    }
                }
            }
            TextArea {
                class: "min-h-12",
                placeholder: "what this project wants it for, e.g. small fixes and questions",
                aria_label: "What kit {position + 1} is for",
                value: row.description.clone(),
                oninput: {
                    let row = row.clone();
                    move |event: FormEvent| onchange.call(KitDraft { description: event.value(), ..row.clone() })
                },
            }
            FittedEditor {
                fitted: row.fitted.clone(),
                shapes,
                available,
                onchange: {
                    let row = row.clone();
                    move |fitted| onchange.call(KitDraft { fitted, ..row.clone() })
                },
            }
            if let Some(problem) = problem {
                p { role: "alert", class: "text-xs text-failed", "{problem}" }
            }
        }
    }
}

/// One variable's row: its name, its value, and the way to take it away.
#[component]
fn Variable(
    position: usize,
    row: VariableDraft,
    kept: bool,
    problem: Option<String>,
    onchange: EventHandler<VariableDraft>,
    onremove: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "flex flex-col gap-1",
            div { class: "flex items-center gap-2",
                input {
                    class: "{FIELD} font-mono",
                    placeholder: "STRIPE_API_KEY",
                    aria_label: "Name of variable {position + 1}",
                    value: "{row.name}",
                    oninput: {
                        let row = row.clone();
                        move |event| onchange.call(VariableDraft { name: event.value(), ..row.clone() })
                    },
                }
                input {
                    r#type: "password",
                    class: FIELD,
                    // Per row rather than per form, because *this row* is
                    // what decides it: a box says "keep" only where there
                    // is something to keep, which is a name the project
                    // already holds.
                    placeholder: if kept { "leave empty to keep" } else { "its value" },
                    aria_label: "Value of variable {position + 1}",
                    value: "{row.value}",
                    oninput: move |event| {
                        onchange.call(VariableDraft { value: event.value(), ..row.clone() });
                    },
                }
                Tooltip { text: "Remove",
                    Button {
                        variant: ButtonVariant::Secondary,
                        class: BESIDE,
                        aria_label: "Remove variable {position + 1}",
                        onclick: move |_| onremove.call(()),
                        {Icon::Remove.draw(16)}
                    }
                }
            }
            if let Some(problem) = problem {
                p { role: "alert", class: "text-xs text-failed", "{problem}" }
            }
        }
    }
}

/// The three choices that set an agent: which one, which model, and how
/// hard it thinks where the model allows a choice — each a row of options,
/// because each is a handful and a person should see them all.
///
/// **Controlled**, like the form around it: it emits the whole [`Fitted`] on
/// every change rather than writing anywhere, so that the parent decides which
/// row it lands in. Moving to another agent starts from that agent's defaults;
/// moving to another model keeps the effort where the new one takes it and
/// clears it where it does not — see [`with_agent`] and [`with_model`], which
/// are pure so that both rules can be tested without a browser.
///
/// The effort appears only where the model takes one. A choice for a setting
/// the model does not have would be offering something the far side refuses.
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
        div { class: "flex flex-wrap items-center gap-2",
            Segmented {
                label: "Agent",
                options: available.iter().map(|agent| (agent.id.clone(), agent.name.clone())).collect::<Vec<_>>(),
                value: fitted.agent.clone(),
                onchange: move |agent: String| onchange.call(with_agent(&shapes, &agent)),
            }
            if let Some(shape) = shape {
                Segmented {
                    label: "Model",
                    options: shape.models.iter().map(|model| (model.id.clone(), model.name.clone())).collect::<Vec<_>>(),
                    value: fitted.model.clone(),
                    onchange: {
                        let fitted = fitted.clone();
                        let shape = shape.clone();
                        move |model: String| onchange.call(with_model(&fitted, &shape, &model))
                    },
                }
                if with_effort {
                    Segmented {
                        label: "Effort",
                        options: shape.efforts.iter().map(|effort| (effort.id.clone(), effort.name.clone())).collect::<Vec<_>>(),
                        value: fitted.effort.clone(),
                        onchange: move |effort: String| {
                            onchange.call(Fitted {
                                effort,
                                ..fitted.clone()
                            });
                        },
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::agents_view::Agent;
    use super::{
        Draft, Filling, Fitted, KitDraft, Shape, Watching, seeded, shape_for, starting,
        takes_effort, with_agent, with_model,
    };
    use stageman_wire::{Choice, ModelChoice};

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

    /// A new project starts on the first agent as it comes, with one kit
    /// named after it, so it can be saved as it opens; an existing one
    /// starts as it is, with its credentials blank, because none ever
    /// reaches a browser.
    #[test]
    fn a_page_starts_from_what_is_true() {
        let watching = Watching {
            projects: vec![stageman_wire::Project {
                id: "p".to_owned(),
                name: "aviary".to_owned(),
                repository: "https://github.com/owner/aviary".to_owned(),
                repository_link: Some("https://github.com/owner/aviary".to_owned()),
                foreman: as_it_comes(),
                kits: vec![KitDraft {
                    name: "deep".to_owned(),
                    description: "big work".to_owned(),
                    fitted: as_it_comes(),
                }],
                platforms: vec!["github".to_owned()],
                channels: vec!["Slack".to_owned()],
                variables: vec!["STRIPE_API_KEY".to_owned()],
                brief: "be careful".to_owned(),
                watched: Vec::new(),
                foreman_room: None,
                attending: false,
                working: 0,
                jobs: 0,
            }],
            available: vec![Agent {
                id: "claude".to_owned(),
                name: "Claude".to_owned(),
                description: "does the work".to_owned(),
                configured: true,
                used_by: Vec::new(),
            }],
            shapes: vec![claude()],
        };

        let fresh = starting(&watching, &Filling::Creating);
        assert_eq!(fresh.foreman, as_it_comes());
        assert_eq!(fresh.kits.len(), 1);
        assert_eq!(
            fresh.kits.first().map(|kit| kit.name.as_str()),
            Some("Claude")
        );
        assert!(fresh.name.is_empty() && fresh.credential.is_empty());

        let existing = starting(&watching, &Filling::Amending("p".to_owned()));
        assert_eq!(existing.name, "aviary");
        assert_eq!(existing.brief, "be careful");
        assert_eq!(
            existing.kits.first().map(|kit| kit.name.as_str()),
            Some("deep")
        );
        assert!(existing.credential.is_empty(), "never seeded");
        assert_eq!(
            existing
                .variables
                .first()
                .map(|row| (row.name.as_str(), row.value.as_str())),
            Some(("STRIPE_API_KEY", "")),
            "the name, with an empty value that means keep"
        );

        let unknown = starting(&watching, &Filling::Amending("q".to_owned()));
        assert_eq!(unknown, Draft::default());
    }
}

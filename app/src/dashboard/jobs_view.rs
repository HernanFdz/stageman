//! One project, and the jobs it has had.
//!
//! The first screen that makes something *happen* rather than configuring what
//! could. Everything before it described an instance; this starts a job, which
//! is one agent in one container doing work on one repository until it is done.
//!
//! **What an operator types is the work, never the instruction.** The agent's
//! instruction is composed by the foreman from the work and the
//! repository, and carries three things that are not negotiable — nothing is
//! checked out, the tools are already authenticated, and work ends at a
//! proposal. `docs/architecture.md` §1 puts every place an instruction is
//! authored in that one crate, which is what makes the snapshot-testing rule
//! in `docs/conventions.md` §4 mean anything at all. A form collecting a
//! finished instruction would route around all of it.

use dioxus::prelude::*;
#[cfg(feature = "server")]
use stageman_instance::{Request, Response};

use super::error::{DashboardError, DashboardResult};
use super::live::Live;
use crate::ui::{
    Badge, BadgeTone, Button, ButtonVariant, Card, EmptyState, Icon, Modal, Reference, Skeleton,
    TextArea, Tooltip, When,
};

pub use stageman_wire::{Ending, Job, Offered, Standing, Working};

/// How a standing reads on a badge.
///
/// A trait rather than a method, because the standing is a wire type and the
/// tone is this crate's: the wire crate names nothing, and a colour token is
/// the dashboard's vocabulary — see
/// `docs/decisions/0026-the-dashboards-vocabulary-is-a-token-set.md`.
pub trait Toned {
    /// The tone this wears.
    fn tone(&self) -> BadgeTone;
}

impl Toned for Standing {
    /// **Tones repeat and labels do not**, which is the deliberate half. There
    /// are four tones and nine standings, so the tone says which family this
    /// is in — going, stopped, wrong, over — and the label says which member.
    /// A tone each would need five more colour tokens to separate things a
    /// reader already tells apart by reading the word.
    fn tone(&self) -> BadgeTone {
        match self {
            Self::Working => BadgeTone::Working,
            Self::Asked | Self::Proposed | Self::Paused | Self::Idle => BadgeTone::Idle,
            // `Lost` is a failure that happens to be final, so it wears the
            // colour a person scans for rather than the one meaning *over*.
            Self::Failed { .. } | Self::Lost => BadgeTone::Failed,
            Self::Done | Self::Discarded => BadgeTone::Neutral,
        }
    }
}

/// Everything one project's screen shows.
///
/// # Errors
///
/// Fails if nothing is watched under that identifier.
#[get("/api/projects/{project}/jobs")]
pub async fn jobs(project: String) -> DashboardResult<Working> {
    match super::ask(Request::Jobs { project }).await? {
        Response::Jobs(working) => Ok(working),
        other => Err(super::unexpected(&other)),
    }
}

/// Starts a job on a project.
///
/// Answered as soon as the record has landed, with the job already in it; the
/// turn runs on a task of the world's own, off the request path as
/// `docs/conventions.md` §3 asks. A person pressing a button has no separate
/// judgement to record, so the reason is filled in by the instance rather
/// than asked for.
///
/// # Errors
///
/// Fails if the project is unknown, if the work is empty, or if the project
/// offers no kit under that name.
#[post("/api/projects/{project}/jobs/start")]
pub async fn start(project: String, kit: String, work: String) -> DashboardResult<Working> {
    match super::ask(Request::Start {
        project,
        kit,
        work,
        at: stageman_core::Timestamp::now(),
    })
    .await?
    {
        Response::Jobs(working) => Ok(working),
        other => Err(super::unexpected(&other)),
    }
}

/// Stops the turn running in a job, keeping everything.
///
/// Answered at once: the job stays working until the world says the turn
/// ended, which is what keeps the reply gate honest. A job whose turn ended
/// while the request was in flight is not an error.
///
/// # Errors
///
/// Fails if the project or the job is unknown.
#[post("/api/projects/{project}/jobs/{job}/stop")]
pub async fn stop(project: String, job: String) -> DashboardResult<Working> {
    match super::ask(Request::Stop { project, job }).await? {
        Response::Jobs(working) => Ok(working),
        other => Err(super::unexpected(&other)),
    }
}

/// Ends a job, and reclaims everything it was holding.
///
/// # Errors
///
/// Fails if the project or the job is unknown, or if a turn is running in it.
#[post("/api/projects/{project}/jobs/{job}/retire")]
pub async fn retire(project: String, job: String, ending: Ending) -> DashboardResult<Working> {
    match super::ask(Request::Retire {
        project,
        job,
        ending,
    })
    .await?
    {
        Response::Jobs(working) => Ok(working),
        other => Err(super::unexpected(&other)),
    }
}

/// One project's screen.
#[component]
pub fn ProjectJobsView(project: String) -> Element {
    // `use_reactive!` because `project` is a plain value rather than a signal:
    // without it this resource keeps its first identifier when the route
    // changes, and the screen shows another project's jobs while claiming to
    // be this one.
    let live = use_context::<Live>();
    let mut reading = use_server_future(use_reactive!(|project| {
        let _ = live.follow();
        jobs(project)
    }))?;
    let mut failure = use_signal(|| None::<DashboardError>);
    let mut starting = use_signal(|| false);
    let mut draft = use_signal(Wanted::default);
    let identifier = project;

    rsx! {
        div { class: "flex flex-col gap-4",
            match reading.cloned() {
                Some(Ok(working)) => rsx! {
                    Card {
                        title: working.name.clone(),
                        badge: rsx! {
                            Badge { "{working.jobs.len()}" }
                            Reference {
                                mark: "github",
                                says: working.repository.clone(),
                                link: working.repository_link.clone(),
                            }
                        },
                        aside: rsx! {
                            div { class: "flex items-center gap-2",
                            Tooltip { text: "Settings",
                                Link {
                                    to: super::Route::ProjectSettingsView {
                                        project: identifier.clone(),
                                    },
                                    class: ButtonVariant::Secondary.styled("px-2"),
                                    aria_label: "Settings",
                                    {Icon::Edit.draw(16)}
                                }
                            }
                            Tooltip { text: "Start a job",
                            Button {
                                class: "px-2",
                                aria_label: "Start a job",
                                onclick: {
                                    // The first kit the project offers, which
                                    // is the one a select with a single option
                                    // would have chosen anyway.
                                    let first = working.kits.first().map(|kit| kit.name.clone());
                                    move |_| {
                                        draft.set(Wanted {
                                            kit: first.clone().unwrap_or_default(),
                                            work: String::new(),
                                        });
                                        failure.set(None);
                                        starting.set(true);
                                    }
                                },
                                {Icon::Add.draw(16)}
                            }
                            }
                            }
                        },
                        if working.jobs.is_empty() {
                            EmptyState {
                                title: "Nothing has run on this project yet.",
                                note: "Describe a piece of work and an agent will do it in a \
                                       container of its own, stopping at a proposal.",
                            }
                        } else {
                            ul { class: "divide-y divide-border",
                                for job in working.jobs {
                                    li { key: "{job.id}",
                                        RanJob {
                                            job,
                                            project: identifier.clone(),
                                            // The child awaits and this
                                            // decides what the screen does
                                            // with the answer, so a row needs
                                            // to know nothing about how the
                                            // page holds its state.
                                            onchanged: move |answered| match answered {
                                                Ok(fresh) => {
                                                    failure.set(None);
                                                    reading.set(Some(Ok(fresh)));
                                                }
                                                Err(reason) => failure.set(Some(reason)),
                                            },
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if starting() {
                        Modal {
                            title: "Start a job",
                            onclose: move |()| starting.set(false),
                            actions: rsx! {
                                Tooltip { text: "Start",
                                Button {
                                    class: "px-2",
                                    aria_label: "Start",
                                    disabled: !draft().is_complete(),
                                    onclick: move |_| {
                                        let identifier = identifier.clone();
                                        let asked = draft();
                                        async move {
                                            match start(identifier, asked.kit, asked.work).await {
                                                Ok(fresh) => {
                                                    failure.set(None);
                                                    reading.set(Some(Ok(fresh)));
                                                    starting.set(false);
                                                }
                                                Err(reason) => failure.set(Some(reason)),
                                            }
                                        }
                                    },
                                    {Icon::Save.draw(16)}
                                }
                                }
                            },
                            if let Some(reason) = failure() {
                                p { class: "mb-3 text-sm text-failed", "{reason}" }
                            }
                            JobForm { draft, kits: working.kits }
                        }
                    }
                },
                Some(Err(reason)) => rsx! {
                    Card { title: "This project could not be read",
                        p { class: "text-sm text-failed", "{reason}" }
                    }
                },
                None => rsx! { Skeleton {} },
            }
        }
    }
}

/// How every icon-only control on a job's row or page is padded.
///
/// Written once because there are several of them: padded and pulled back,
/// so the target is bigger than the shape without moving anything around
/// it. The colour and the hover are the ghost button's.
const CONTROL: &str = "-m-1 p-1";

/// One job, as the list shows it: its standing, its reason as the way to
/// its page, what it ran on and when, and the controls its standing offers.
#[component]
fn RanJob(
    job: Job,
    project: String,
    onchanged: EventHandler<Result<Working, DashboardError>>,
) -> Element {
    rsx! {
        div { class: "flex flex-col gap-1.5 py-4 first:pt-0 last:pb-0",
            div { class: "flex items-baseline gap-3",
                Badge { tone: job.standing.tone(), "{job.standing.label()}" }
                // The reason is the way to the job's page, where its
                // instruction and its links are — see
                // `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`.
                Link {
                    to: super::Route::ProjectJobView {
                        project: project.clone(),
                        job: job.id.clone(),
                    },
                    class: "text-sm hover:underline",
                    "{job.reason}"
                }
                for opened in job.pull_requests.iter() {
                    super::job_view::PullRequestChip { key: "{opened.number}", number: opened.number, link: opened.link.clone() }
                }
                span { class: "ml-auto flex shrink-0 items-baseline gap-2 font-mono text-xs text-faint-foreground",
                    "{job.kit}"
                    When { at: job.created_at.clone() }
                }
            }
            if let Standing::Failed { why } = &job.standing {
                p { class: "text-xs text-failed", "{why}" }
            }
            // What the session said it was set to, in the adapter's spelling,
            // beside what was asked for above. Shown whenever there is
            // anything, because the one case worth seeing is the two
            // disagreeing — and a reader cannot spot a disagreement that is
            // only shown when it occurs.
            if !job.reported.is_empty() {
                p { class: "font-mono text-xs text-faint-foreground",
                    "reported "
                    {job.reported.iter().map(|(option, value)| format!("{option} {value}")).collect::<Vec<_>>().join(" · ")}
                }
            }
            div { class: "flex items-center gap-1",
                Showing { tunnel: job.tunnel.clone() }
                JobControls { project, job, onchanged }
            }
        }
    }
}

/// The way to what a job is showing, as an arrow leaving a frame.
///
/// In a tab of its own, and told to carry nothing there. What is on the
/// other side is an application this instance's agent wrote, so it gets
/// neither a handle on the page that opened it nor the address that page
/// was at. An arrow leaving a frame rather than an eye, and the distinction
/// is worth keeping: an eye means *reveal this*, and this one navigates
/// away.
#[component]
pub(super) fn Showing(tunnel: String) -> Element {
    rsx! {
        Tooltip { text: "Look at what it is showing — {tunnel}",
            a {
                class: "{CONTROL} inline-flex items-center rounded-md text-muted-foreground \
                        hover:bg-surface-muted hover:text-foreground focus-visible:outline-none \
                        focus-visible:ring-2 focus-visible:ring-primary",
                href: "{tunnel}",
                target: "_blank",
                rel: "noopener noreferrer",
                aria_label: "Look at what it is showing",
                {Icon::Look.draw(16)}
            }
        }
    }
}

/// The controls a job offers, on its row and on its page.
///
/// **Which controls it offers is decided by the standing**, and the two are
/// deliberately never offered together: a working job can be stopped and not
/// retired, and every other job can be retired and not stopped. Retiring
/// destroys the container and the session in it, so putting it beside a
/// control that keeps both would make the irreversible one a mis-click away —
/// see `docs/decisions/0053-a-job-is-stopped-or-retired-by-a-person.md`.
///
/// A job that is already over offers neither: there is nothing left to stop
/// and nothing left to reclaim.
#[component]
pub(super) fn JobControls(
    project: String,
    job: Job,
    onchanged: EventHandler<Result<Working, DashboardError>>,
) -> Element {
    // Which ending is being confirmed, if one is. Retiring removes the
    // container and the session in it, so it is asked twice — see
    // `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`.
    let mut confirming = use_signal(|| None::<Ending>);
    let over = job.standing.is_over();

    rsx! {
        // Icons rather than words, because a row of jobs is a list and a
        // list reads better as shapes. Every control carries an accessible
        // name, and its tooltip repeats the name for eyes: an icon-only
        // control with neither is a puzzle.
        div { class: "flex items-center gap-1",
            // A working job can be stopped, and that is all it can be:
            // its container is in use and its session is mid-turn.
            if job.standing == Standing::Working {
                Tooltip { text: "Stop it — the job keeps everything and can be given more",
                    Button {
                        variant: ButtonVariant::Ghost,
                        class: CONTROL,
                        aria_label: "Stop it",
                        onclick: {
                            let project = project.clone();
                            let id = job.id.clone();
                            move |_| {
                                let project = project.clone();
                                let id = id.clone();
                                async move { onchanged.call(stop(project, id).await) }
                            }
                        },
                        {Icon::Stop.draw(16)}
                    }
                }
            }
            // And a job that has stopped can be ended, either way. Two
            // controls rather than one with a choice behind it, because
            // the verdict is the whole of what is being recorded.
            if !over && job.standing != Standing::Working {
                Tooltip { text: "It is done — removes its container and everything in it",
                    Button {
                        variant: ButtonVariant::Ghost,
                        class: CONTROL,
                        aria_label: "It is done",
                        onclick: move |_| confirming.set(Some(Ending::Done)),
                        {Icon::Done.draw(16)}
                    }
                }
                Tooltip { text: "Discard it — removes its container and everything in it",
                    Button {
                        variant: ButtonVariant::Ghost,
                        class: CONTROL,
                        aria_label: "Discard it",
                        onclick: move |_| confirming.set(Some(Ending::Discarded)),
                        {Icon::Discard.draw(16)}
                    }
                }
            }
            if let Some(ending) = confirming() {
                Modal {
                    title: match ending {
                        Ending::Done => "Retire this job as done?",
                        Ending::Discarded => "Discard this job?",
                    },
                    onclose: move |()| confirming.set(None),
                    actions: rsx! {
                        Button {
                            variant: match ending {
                                Ending::Done => ButtonVariant::Primary,
                                Ending::Discarded => ButtonVariant::Danger,
                            },
                            onclick: {
                                // Moved rather than cloned: the last
                                // control on the row is the last thing
                                // that wants it.
                                let project = project;
                                let id = job.id;
                                move |_| {
                                    let project = project.clone();
                                    let id = id.clone();
                                    confirming.set(None);
                                    async move {
                                        onchanged.call(retire(project, id, ending).await);
                                    }
                                }
                            },
                            match ending {
                                Ending::Done => "Retire",
                                Ending::Discarded => "Discard",
                            }
                        }
                    },
                    p { class: "text-sm text-muted-foreground",
                        "Its container and everything in it are removed, and the job stays in \
                         the list as "
                        match ending {
                            Ending::Done => "done",
                            Ending::Discarded => "discarded",
                        }
                        ". Nothing on the platform changes."
                    }
                }
            }
            if over {
                Tooltip { text: "This job is over — its container and session are gone",
                    span {
                        class: "{CONTROL} inline-flex text-faint-foreground",
                        // Reachable by keyboard, so the tooltip can be
                        // asked for without a mouse: nothing else on the
                        // row says what the shape means.
                        tabindex: "0",
                        aria_label: "This job is over",
                        {Icon::Over.draw(16)}
                    }
                }
            }
        }
    }
}

/// What starting a job asks for.
///
/// The work and which kit, and nothing else. Not the instruction: that is
/// composed from this, and composing it here would put an author of
/// instructions outside the one crate allowed to be one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Wanted {
    /// Which of the project's kits should do it, by name.
    pub kit: String,
    /// What to do, in the operator's own words.
    pub work: String,
}

impl Wanted {
    /// Whether this says enough to start a job.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        !self.kit.trim().is_empty() && !self.work.trim().is_empty()
    }
}

/// The form that describes a piece of work.
///
/// Controlled, like the project form and for the same reason: the control that
/// submits lives in the modal's header, and cannot read state the form owns.
#[component]
fn JobForm(draft: Signal<Wanted>, kits: Vec<Offered>) -> Element {
    let mut draft = draft;

    rsx! {
        div { class: "flex flex-col gap-3",
            // Only when there is a choice. A project with one kit has already
            // made this decision, and a choice with one option asks a question
            // that has no other answer. Cards rather than a dropdown, per
            // `docs/conventions.md` §3: a kit is chosen by what it is for,
            // and that is a line to read, not an entry to scroll past.
            if kits.len() > 1 {
                div { class: "flex flex-col gap-1", role: "radiogroup", aria_label: "Runs on",
                    span { class: "text-xs font-medium text-muted-foreground", "Runs on" }
                    for kit in kits.iter() {
                        label {
                            key: "{kit.name}",
                            class: if draft().kit == kit.name {
                                "flex cursor-pointer items-baseline gap-2 rounded-md border border-primary bg-surface-muted px-3 py-2"
                            } else {
                                "flex cursor-pointer items-baseline gap-2 rounded-md border border-border px-3 py-2 hover:bg-surface-muted"
                            },
                            input {
                                r#type: "radio",
                                name: "kit",
                                class: "sr-only",
                                value: "{kit.name}",
                                checked: draft().kit == kit.name,
                                onchange: {
                                    let name = kit.name.clone();
                                    move |_| draft.with_mut(|draft| draft.kit.clone_from(&name))
                                },
                            }
                            span { class: "text-sm font-medium", "{kit.name}" }
                            span { class: "text-xs text-muted-foreground", "{kit.description}" }
                        }
                    }
                }
            }
            label { class: "flex flex-col gap-1",
                span { class: "text-xs font-medium text-muted-foreground", "The work" }
                TextArea {
                    class: "min-h-40",
                    placeholder: "What needs doing, in your own words. Say what \"done\" looks \
                                  like, and name anything the agent should read first.",
                    value: draft().work,
                    oninput: move |event: FormEvent| draft.with_mut(|draft| draft.work = event.value()),
                }
            }
            p { class: "text-xs text-faint-foreground",
                "The agent is told where the repository is, that nothing is checked out, that \
                 its tools are already signed in, and to stop at a proposal rather than merge \
                 anything. You are describing the work, not writing the instruction."
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Standing, Toned as _, Wanted};
    use crate::ui::BadgeTone;

    /// Every standing there is.
    ///
    /// Listed by hand: a variant added without a line here is one nothing
    /// below checks.
    fn every() -> Vec<Standing> {
        vec![
            Standing::Working,
            Standing::Asked,
            Standing::Proposed,
            Standing::Paused,
            Standing::Idle,
            Standing::Failed {
                why: "it did not work".to_owned(),
            },
            Standing::Done,
            Standing::Discarded,
            Standing::Lost,
        ]
    }

    /// A running job and a failed one being indistinguishable is the failure
    /// a tone exists to prevent, and this is where the two are decided.
    #[test]
    fn no_two_standings_look_or_read_alike() {
        let labels: Vec<&str> = every().iter().map(Standing::label).collect();

        // Tones are shared on purpose and labels are not, so what has to be
        // unique is the word. This used to demand a tone each, which held only
        // while there were as many tones as standings.
        for standing in every() {
            let alarming = matches!(standing.tone(), BadgeTone::Failed);
            let wrong = matches!(standing, Standing::Failed { .. } | Standing::Lost);
            assert_eq!(
                alarming, wrong,
                "{standing:?} wears the colour a person scans for, or fails to",
            );
        }
        assert!(labels.iter().all(|label| !label.is_empty()));
        assert_eq!(
            labels
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            labels.len(),
            "two standings share a label: {labels:?}"
        );
    }

    /// The control that starts a job is unavailable until pressing it would
    /// work, so this is the whole of that guard.
    #[test]
    fn a_request_needs_both_a_kit_and_some_work() {
        let complete = Wanted {
            kit: "Claude".to_owned(),
            work: "document the three missing variables".to_owned(),
        };
        assert!(complete.is_complete());

        let mut without_kit = complete.clone();
        without_kit.kit.clear();
        assert!(!without_kit.is_complete());

        let mut without_work = complete;
        without_work.work.clear();
        assert!(!without_work.is_complete());
    }

    /// Whitespace is not a description of work.
    ///
    /// The route trims before judging, so a form accepting spaces would offer
    /// a control that fails.
    #[test]
    fn whitespace_is_not_work() {
        let asked = Wanted {
            kit: "Claude".to_owned(),
            work: "  \n\t ".to_owned(),
        };

        assert!(!asked.is_complete());
    }
}

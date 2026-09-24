//! A person at the dashboard: every request is answered once whatever it
//! changed is on the disk, and refused with a reason the screen can show.

use stageman_agent::Command;
use stageman_channel::Call;
use stageman_core::{Agent, JobId, Outcome, Progress, ProjectId, Timestamp, Uuid, Waiting};
use stageman_instance::{Instance, Request, Response};
use stageman_platform::Call as PlatformCall;
use stageman_wire::{
    AccessDraft, AccessView, ChannelDraft, Draft, Ending, Fitted, KitDraft, Refusal, Standing,
};

use crate::simulation::{
    Simulation, job, project, request, room, seed, watching, watching_a_channel,
};

/// Asks, performs, and lets the write land and the answer follow.
pub fn ask(sim: &mut Simulation, instance: &mut Instance, id: u64, asked: Request) -> Response {
    for effect in instance.step(sim.now(), request(id, asked)) {
        sim.perform(effect);
    }
    let until = sim.now() + 5;
    sim.run_until(instance, until);
    sim.response(id)
        .cloned()
        .expect("every request is answered")
}

fn as_it_comes() -> Fitted {
    Fitted {
        agent: "claude".to_owned(),
        model: "default".to_owned(),
        effort: "default".to_owned(),
    }
}

/// A project as a person drafts one, bound to a room of its own name, since
/// a project is created with a binding or not at all.
pub fn a_draft(name: &str) -> Draft {
    Draft {
        name: name.to_owned(),
        foreman: as_it_comes(),
        kits: vec![KitDraft {
            name: "Claude".to_owned(),
            description: "General-purpose.".to_owned(),
            fitted: as_it_comes(),
        }],
        // An address on the platform, since a draft with anything else is
        // refused before it becomes a project.
        access: AccessDraft::Token {
            token: Some("ghp-not-a-real-token".to_owned()),
            repository: Some(format!("https://github.com/example/{name}")),
        },
        channel: ChannelDraft {
            credential: "xoxb-not-a-real-token".to_owned(),
            listen_credential: "xapp-not-a-real-token".to_owned(),
        },
        brief: String::new(),
        variables: Vec::new(),
    }
}

/// Where in the trace a line first mentions something.
/// Where in the trace a container was first told to go.
///
/// Every runtime command is one generic effect now, so what tells them
/// apart is the argument list, read back through the agent crate's own
/// inverse rather than matched as a string.
fn first_removal(sim: &Simulation) -> usize {
    let found = sim.first_asking(|command| matches!(command, Command::Discard { .. }));
    assert!(
        found.is_some(),
        "nothing was removed: {:#?}",
        sim.commands()
    );
    found.expect("asserted above")
}

/// How many containers were told to go.
fn removals(sim: &Simulation) -> usize {
    sim.commands()
        .iter()
        .filter(|command| matches!(command, Command::Discard { .. }))
        .count()
}

pub fn first(sim: &Simulation, what: &str) -> usize {
    let found = sim.trace().iter().position(|line| line.contains(what));
    assert!(
        found.is_some(),
        "nothing in the trace says {what}: {:#?}",
        sim.trace()
    );
    found.expect("asserted above")
}

pub fn count(sim: &Simulation, what: &str) -> usize {
    sim.trace()
        .iter()
        .filter(|line| line.contains(what))
        .count()
}

/// Where in the trace the n-th line mentioning something is, counting from
/// nought: what tells a request's own write from the wake's.
pub fn nth(sim: &Simulation, what: &str, n: usize) -> usize {
    let found = sim
        .trace()
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains(what))
        .nth(n)
        .map(|(at, _)| at);
    assert!(
        found.is_some(),
        "fewer than {} lines in the trace say {what}: {:#?}",
        n + 1,
        sim.trace()
    );
    found.expect("asserted above")
}

#[test]
fn the_instance_screen_counts_what_it_may_and_never_a_credential() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));
    let written = count(&sim, "-> Write");

    let Response::Instance(shown) = ask(&mut sim, &mut instance, 1, Request::Instance) else {
        panic!("the status line");
    };
    // The first candidate on the platform every scenario is played on.
    assert_eq!(shown.container_runtime, "/usr/bin/docker");
    assert_eq!(shown.agents, 1);
    assert!(!shown.domain.is_empty() && !shown.version.is_empty());
    let served = serde_json::to_string(&shown).expect("it serialises");
    assert!(!served.contains("agent-token"), "{served}");

    let Response::Home(shown) = ask(&mut sim, &mut instance, 2, Request::Home) else {
        panic!("the first page");
    };
    assert_eq!(shown.projects.len(), 1);
    assert!(shown.needs_you.is_empty() && shown.working.is_empty());
    let served = serde_json::to_string(&shown).expect("it serialises");
    assert!(!served.contains("agent-token"), "{served}");
    assert_eq!(count(&sim, "-> Write"), written, "a read writes nothing");
}

/// The answer follows the write, and a refusal changes nothing.
#[test]
fn configuring_an_agent_is_answered_once_the_credential_has_landed() {
    let mut sim = Simulation::new();
    let mut instance = sim.wake(seed(1));
    assert!(sim.disk().is_none(), "an instance starts empty");

    let Response::Agents(agents) = ask(
        &mut sim,
        &mut instance,
        1,
        Request::Configure {
            agent: "claude".to_owned(),
            credential: " a-new-token ".to_owned(),
        },
    ) else {
        panic!("the agents screen");
    };
    assert!(
        agents
            .iter()
            .any(|agent| agent.id == "claude" && agent.configured)
    );
    assert!(first(&sim, "-> Write") < first(&sim, "-> Respond"));
    let landed = sim.disk().expect("the write landed");
    assert_eq!(
        landed
            .agents
            .get(&Agent::Claude)
            .map(|config| config.auth_token.expose()),
        Some("a-new-token")
    );

    let written = count(&sim, "-> Write");
    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            2,
            Request::Configure {
                agent: "claude".to_owned(),
                credential: "   ".to_owned(),
            },
        ),
        Response::Refused(Refusal::CredentialMissing)
    );
    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            3,
            Request::ForgetAgent {
                agent: "gpt".to_owned(),
            },
        ),
        Response::Refused(Refusal::UnknownAgent {
            name: "gpt".to_owned()
        })
    );
    assert_eq!(count(&sim, "-> Write"), written, "refusals write nothing");

    let Response::Agents(agents) = ask(
        &mut sim,
        &mut instance,
        4,
        Request::ForgetAgent {
            agent: "claude".to_owned(),
        },
    ) else {
        panic!("the agents screen");
    };
    assert!(agents.iter().all(|agent| !agent.configured));
    assert!(sim.disk().expect("landed").agents.is_empty());
}

#[test]
fn an_agent_a_project_names_cannot_be_forgotten() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));

    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            1,
            Request::ForgetAgent {
                agent: "claude".to_owned(),
            },
        ),
        Response::Refused(Refusal::AgentInUse {
            agent: "claude".to_owned(),
            projects: vec!["example".to_owned()],
        })
    );
    assert!(instance.state().agents.contains_key(&Agent::Claude));
}

/// A project bound to a channel is listened on from the moment its record
/// has landed.
#[test]
fn creating_a_project_listens_on_its_channel_once_the_record_has_landed() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));
    assert_eq!(sim.listening(), 0, "the one project has no channel");

    let mut draft = a_draft("burrow");
    draft.channel = ChannelDraft {
        credential: "xoxb-not-a-real-token".to_owned(),
        listen_credential: "xapp-not-a-real-token".to_owned(),
    };
    let writes_before = count(&sim, "-> Write");
    let Response::Projects(shown) = ask(&mut sim, &mut instance, 1, Request::Create { draft })
    else {
        panic!("the projects screen");
    };
    assert_eq!(shown.projects.len(), 2);
    let burrow = shown
        .projects
        .iter()
        .find(|project| project.name == "burrow")
        .expect("the new project");
    assert_eq!(burrow.channels, vec!["Slack".to_owned()]);
    assert_eq!(burrow.access, Some(AccessView::Token));
    let created = ProjectId::from_uuid(Uuid::parse_str(&burrow.id).expect("an identifier"));
    assert_eq!(
        sim.listening(),
        1,
        "listened to from the moment the record landed"
    );
    // The token and both of the binding's credentials were checked before
    // anything was written — see
    // `docs/decisions/0076-a-credential-is-guided-in-and-checked-before-it-is-kept.md`
    // — and listening then begins by asking the platform who this instance
    // is again, only once the record has landed.
    let written = nth(&sim, "-> Write", writes_before);
    let read = sim
        .platform_calls()
        .iter()
        .find(|(_, call)| matches!(call, PlatformCall::Repository { .. }))
        .map(|(at, _)| *at)
        .expect("the token was checked against the repository");
    let introduced: Vec<usize> = sim
        .channel_calls()
        .iter()
        .filter(|(_, call)| matches!(call, Call::WhoAmI { .. }))
        .map(|(at, _)| *at)
        .collect();
    let [checked, listening] = introduced.as_slice() else {
        panic!("checked, then introduced: {introduced:?}");
    };
    assert!(
        read < written && *checked < written,
        "checked before written"
    );
    assert!(written < *listening, "introduced once the record landed");
    assert!(*listening < first(&sim, "-> Respond"));
    assert!(sim.disk().expect("landed").projects.contains_key(&created));

    let mut blank = a_draft("blank");
    blank.name = "  ".to_owned();
    assert_eq!(
        ask(&mut sim, &mut instance, 3, Request::Create { draft: blank }),
        Response::Refused(Refusal::Incomplete {
            field: "name".to_owned()
        })
    );
    assert_eq!(
        instance.state().projects.len(),
        2,
        "refusals change nothing"
    );
}

/// A new project's identifier comes from the seed, so the same scenario
/// names the same project every time.
#[test]
fn a_created_project_is_named_by_the_seed() {
    let created = |seed_of: u8| {
        let mut sim = Simulation::new();
        sim.holding(&watching(&[]));
        let mut instance = sim.wake(seed(seed_of));
        let Response::Projects(shown) = ask(
            &mut sim,
            &mut instance,
            1,
            Request::Create {
                draft: a_draft("burrow"),
            },
        ) else {
            panic!("the projects screen");
        };
        shown
            .projects
            .iter()
            .find(|project| project.name == "burrow")
            .map(|project| project.id.clone())
            .expect("created")
    };
    assert_eq!(created(3), created(3));
    assert_ne!(created(3), created(4));
}

/// A token left unsaid is the one held, so a project holding none is
/// refused it; typed replaces; and what lands is what the screen was told.
#[test]
fn amending_keeps_a_credential_when_the_box_is_blank() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));
    let id = project().to_string();

    let mut blank = a_draft("example");
    blank.access = AccessDraft::Token {
        token: None,
        repository: Some("https://github.com/example/renamed".to_owned()),
    };
    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            1,
            Request::Amend {
                project: id.clone(),
                draft: blank.clone(),
            },
        ),
        Response::Refused(Refusal::Incomplete {
            field: "access".to_owned()
        }),
        "it had none, so none can be left unsaid"
    );

    let mut typed = a_draft("renamed");
    typed.access = AccessDraft::Token {
        token: Some("ghp-the-new-one".to_owned()),
        repository: Some("https://github.com/example/renamed".to_owned()),
    };
    ask(
        &mut sim,
        &mut instance,
        2,
        Request::Amend {
            project: id.clone(),
            draft: typed,
        },
    );
    blank.name = "renamed".to_owned();
    ask(
        &mut sim,
        &mut instance,
        3,
        Request::Amend {
            project: id,
            draft: blank,
        },
    );
    let landed = sim.disk().expect("landed");
    let watched = landed.projects.get(&project()).expect("still watched");
    assert_eq!(watched.name, "renamed");
    assert!(matches!(
        watched.access.get(&stageman_core::Platform::GitHub),
        Some(stageman_core::Access::Token(token)) if token.expose() == "ghp-the-new-one"
    ));

    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            4,
            Request::Amend {
                project: "nobody".to_owned(),
                draft: a_draft("x"),
            },
        ),
        Response::Refused(Refusal::UnknownProject {
            id: "nobody".to_owned()
        })
    );
}

/// A busy project is refused; an idle one goes with its containers, and
/// the containers go only once the record's removal has landed.
#[test]
fn forgetting_a_project_removes_its_containers_and_refuses_while_busy() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[
        (job(1), Progress::Working),
        (job(2), Progress::Idle(Waiting::Silent)),
    ]));
    let (name, held) = Simulation::ours(&stageman_job::container(&job(1)));
    sim.container(&name, held);
    let (name, held) = Simulation::ours(&stageman_job::container(&job(2)));
    sim.container(&name, held);
    let (name, held) = Simulation::ours(&stageman_foreman::container(project()));
    sim.container(&name, held);
    let mut instance = sim.wake(seed(1));
    let id = project().to_string();

    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            1,
            Request::Forget {
                project: id.clone()
            },
        ),
        Response::Refused(Refusal::ProjectBusy {
            name: "example".to_owned(),
            working: 1,
        })
    );

    sim.run_until(&mut instance, 2_000);
    assert!(!instance.state().working().any(|working| working == job(1)));
    let Response::Projects(shown) =
        ask(&mut sim, &mut instance, 2, Request::Forget { project: id })
    else {
        panic!("the projects screen");
    };
    assert!(shown.projects.is_empty());
    assert!(!sim.exists(&stageman_job::container(&job(1))));
    assert!(!sim.exists(&stageman_job::container(&job(2))));
    assert!(!sim.exists(&stageman_foreman::container(project())));
    assert!(first(&sim, "-> Write") < first_removal(&sim));
    assert!(sim.reclaims() >= 1);
    assert!(sim.disk().expect("landed").projects.is_empty());
}

/// A job started by hand is on the record before its turn runs, and its
/// turn runs only once the record has landed.
#[test]
fn starting_a_job_by_hand_runs_its_first_turn_once_the_record_has_landed() {
    let mut sim = Simulation::new();
    sim.holding(&watching_a_channel(&[]));
    let mut instance = sim.wake(seed(1));
    let id = project().to_string();

    let Response::Jobs(shown) = ask(
        &mut sim,
        &mut instance,
        1,
        Request::Start {
            project: id.clone(),
            kit: "Claude".to_owned(),
            work: " fix the build ".to_owned(),
            title: String::new(),
        },
    ) else {
        panic!("the project's screen");
    };
    assert_eq!(shown.jobs.len(), 1);
    let started = &shown.jobs[0];
    assert_eq!(started.standing, Standing::Working);
    assert!(
        started.kickoff.contains("fix the build"),
        "{}",
        started.kickoff
    );
    assert_eq!(started.reason, "started by hand from the dashboard");
    assert!(started.tunnel.contains(&started.id));
    assert!(first(&sim, "-> Write") < sim.first_turn().expect("a turn"));
    let begun = JobId::parse(&started.id).expect("a name");
    assert!(sim.is_running(&stageman_job::container(&begun)));

    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            2,
            Request::Start {
                project: id.clone(),
                kit: "gpt".to_owned(),
                work: "anything".to_owned(),
                title: String::new(),
            },
        ),
        Response::Refused(Refusal::KitNotOnProject {
            name: "gpt".to_owned(),
            project: "example".to_owned(),
        })
    );
    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            3,
            Request::Start {
                project: id,
                kit: "Claude".to_owned(),
                work: "  ".to_owned(),
                title: String::new(),
            },
        ),
        Response::Refused(Refusal::Incomplete {
            field: "work".to_owned()
        })
    );
    assert_eq!(instance.state().projects[&project()].jobs.len(), 1);
}

/// On a project with a channel, the room is made before the turn, and the
/// job's record names it.
#[test]
fn a_job_started_by_hand_on_a_bound_project_has_its_room_made_first() {
    let mut sim = Simulation::new();
    sim.holding(&watching_a_channel(&[]));
    let mut instance = sim.wake(seed(1));

    let Response::Jobs(shown) = ask(
        &mut sim,
        &mut instance,
        1,
        Request::Start {
            project: project().to_string(),
            kit: "Claude".to_owned(),
            work: "fix the build".to_owned(),
            title: String::new(),
        },
    ) else {
        panic!("the project's screen");
    };
    assert_eq!(shown.jobs.len(), 1);
    let made = sim
        .first_call(|call| matches!(call, Call::CreateRoom { .. }))
        .expect("the room was made");
    assert!(made < sim.first_turn().expect("a turn"));
    assert_eq!(sim.rooms().len(), 1);
    let started = JobId::parse(&shown.jobs[0].id).expect("a name");
    assert!(
        instance
            .state()
            .job(&started)
            .and_then(|job| job.room.clone())
            .is_some()
    );
}

/// A job's page says what the job is and links only what is true: its room
/// once the channel has said where its workspace is, and nothing for a job
/// nobody has — see
/// `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`.
#[test]
fn a_jobs_page_says_what_it_is_and_links_only_what_is_true() {
    let mut sim = Simulation::new();
    sim.holding(&watching_a_channel(&[]));
    let mut instance = sim.wake(seed(1));

    let Response::Jobs(shown) = ask(
        &mut sim,
        &mut instance,
        1,
        Request::Start {
            project: project().to_string(),
            kit: "Claude".to_owned(),
            work: "fix the build".to_owned(),
            title: String::new(),
        },
    ) else {
        panic!("the project's screen");
    };
    let started = shown.jobs[0].id.clone();

    let Response::Job(page) = ask(
        &mut sim,
        &mut instance,
        2,
        Request::Job {
            project: project().to_string(),
            job: started.clone(),
        },
    ) else {
        panic!("the job's page");
    };
    assert_eq!(page.job.id, started);
    assert_eq!(page.project, project().to_string());
    assert_eq!(page.job.standing, Standing::Working);
    // What it runs on, with every name a chip needs resolved on this side:
    // the kit a person drafted, read back as it is shown.
    assert_eq!(page.job.kit.agent, as_it_comes().agent);
    assert_eq!(page.job.kit.agent_name, "Claude");
    assert_eq!(page.job.kit.model, "Default");
    assert_eq!(
        page.job.kit.effort,
        Some(("default".to_owned(), "Default".to_owned()))
    );
    assert!(
        page.job.kickoff.contains("fix the build"),
        "{}",
        page.job.kickoff
    );
    let room = page.job.room.clone().expect("a room was made for it");
    assert_eq!(
        page.job.room_link.as_deref(),
        Some(format!("https://example.slack.com/archives/{room}").as_str()),
        "linked from where the channel said its workspace is"
    );

    let refused = ask(
        &mut sim,
        &mut instance,
        3,
        Request::Job {
            project: project().to_string(),
            job: "00000000-0000-0000-0000-00000000dead".to_owned(),
        },
    );
    assert!(
        matches!(refused, Response::Refused(Refusal::UnknownJob { .. })),
        "{refused:?}"
    );
}

/// Stopping ends the turn, and the job is paused once the world says it
/// ended — not before.
#[test]
fn stopping_a_job_ends_its_turn_and_leaves_it_paused() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[
        (job(1), Progress::Working),
        (job(2), Progress::Idle(Waiting::Silent)),
    ]));
    let (name, held) = Simulation::ours(&stageman_job::container(&job(1)));
    sim.container(&name, held);
    let (name, held) = Simulation::ours(&stageman_job::container(&job(2)));
    sim.container(&name, held);
    let mut instance = sim.wake(seed(1));
    // Far enough for the resumed job's agent to be running, so that the
    // stop reaches a process rather than a step still to come.
    sim.run_until(&mut instance, 3);
    let id = project().to_string();

    for effect in instance.step(
        sim.now(),
        request(
            1,
            Request::Stop {
                project: id.clone(),
                job: job(1).to_string(),
            },
        ),
    ) {
        sim.perform(effect);
    }
    let Some(Response::Jobs(shown)) = sim.response(1).cloned() else {
        panic!("answered at once: nothing was written");
    };
    let stopped = shown
        .jobs
        .iter()
        .find(|listed| listed.id == job(1).to_string());
    assert_eq!(
        stopped.map(|job| job.standing.clone()),
        Some(Standing::Working),
        "still working until the world says the turn ended"
    );
    assert_eq!(count(&sim, "-> Close"), 1);

    sim.run_until(&mut instance, 10);
    assert_eq!(
        instance
            .state()
            .job(&job(1))
            .map(|job| job.progress.clone()),
        Some(Progress::Idle(Waiting::Paused))
    );
    // The moment the standing changed is the step's own time, stamped by
    // the world — see
    // `docs/decisions/0073-the-world-tells-the-instance-the-time-with-every-step.md`
    // — which on this clock is after the epoch and no later than now.
    let changed = instance
        .state()
        .job(&job(1))
        .and_then(|job| job.since)
        .expect("a standing that changed has a moment");
    assert!(
        changed > Timestamp::UNIX_EPOCH
            && changed <= Timestamp::from_millisecond(10).expect("a moment"),
        "{changed}"
    );
    assert_eq!(
        sim.disk()
            .expect("landed")
            .job(&job(1))
            .map(|job| job.progress.clone()),
        Some(Progress::Idle(Waiting::Paused))
    );
    assert!(
        sim.exists(&stageman_job::container(&job(1))),
        "stopping keeps"
    );

    ask(
        &mut sim,
        &mut instance,
        2,
        Request::Stop {
            project: id.clone(),
            job: job(2).to_string(),
        },
    );
    assert_eq!(count(&sim, "-> Close"), 1, "nothing to stop in an idle job");
    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            3,
            Request::Stop {
                project: id,
                job: "nope".to_owned(),
            },
        ),
        Response::Refused(Refusal::UnknownJob {
            id: "nope".to_owned()
        })
    );
}

/// The verdict lands before the container goes, a working job is refused,
/// and pressing twice changes nothing but asks the container to go again.
#[test]
fn retiring_a_job_records_the_verdict_before_its_container_goes() {
    let mut sim = Simulation::new();
    sim.holding(&watching_a_channel(&[
        (job(1), Progress::Idle(Waiting::Asked), 1),
        (job(2), Progress::Working, 2),
    ]));
    let (name, held) = Simulation::ours(&stageman_job::container(&job(1)));
    sim.container(&name, held);
    let (name, held) = Simulation::ours(&stageman_job::container(&job(2)));
    sim.container(&name, held);
    let mut instance = sim.wake(seed(1));
    let id = project().to_string();
    let reclaimed = sim.reclaims();

    let Response::Jobs(shown) = ask(
        &mut sim,
        &mut instance,
        1,
        Request::Retire {
            project: id.clone(),
            job: job(1).to_string(),
            ending: Ending::Done,
        },
    ) else {
        panic!("the project's screen");
    };
    let retired = shown
        .jobs
        .iter()
        .find(|listed| listed.id == job(1).to_string())
        .expect("still listed");
    assert_eq!(retired.standing, Standing::Done);
    assert!(first(&sim, "-> Write") < first_removal(&sim));
    assert!(!sim.exists(&stageman_job::container(&job(1))));
    assert_eq!(sim.reclaims(), reclaimed + 1);
    // Its room went with it, once the verdict was on the disk: an archived
    // room leaves the sidebar and takes no more posts.
    assert_eq!(sim.archived(), [room(1).id]);
    let archived = sim
        .first_call(|call| matches!(call, Call::Archive { .. }))
        .expect("the room was archived");
    assert!(first(&sim, "-> Write") < archived);
    assert_eq!(
        sim.disk()
            .expect("landed")
            .job(&job(1))
            .map(|job| job.progress.clone()),
        Some(Progress::Retired(Outcome::Done))
    );

    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            2,
            Request::Retire {
                project: id.clone(),
                job: job(2).to_string(),
                ending: Ending::Discarded,
            },
        ),
        Response::Refused(Refusal::JobWorking)
    );

    ask(
        &mut sim,
        &mut instance,
        3,
        Request::Retire {
            project: id,
            job: job(1).to_string(),
            ending: Ending::Discarded,
        },
    );
    assert_eq!(
        instance
            .state()
            .job(&job(1))
            .map(|job| job.progress.clone()),
        Some(Progress::Retired(Outcome::Done)),
        "a verdict is never overwritten"
    );
    assert_eq!(removals(&sim), 2, "safe to press twice");
}

/// The same requests against the same seed leave the same trace.
#[test]
fn the_same_requests_leave_the_same_trace() {
    let run = || {
        let mut sim = Simulation::new();
        sim.holding(&watching_a_channel(&[]));
        let mut instance = sim.wake(seed(9));
        ask(
            &mut sim,
            &mut instance,
            1,
            Request::Create {
                draft: a_draft("burrow"),
            },
        );
        ask(
            &mut sim,
            &mut instance,
            2,
            Request::Start {
                project: project().to_string(),
                kit: "Claude".to_owned(),
                work: "fix the build".to_owned(),
                title: String::new(),
            },
        );
        sim.run_until(&mut instance, 5_000);
        sim.trace().to_vec()
    };
    assert_eq!(run(), run());
}

/// A job cannot start on a project with no channel bound, which only a
/// project the last release wrote can lack, and the refusal names the
/// project so that the operator knows what to bind — see
/// `docs/decisions/0059-a-project-speaks-and-listens-on-slack-always.md`.
#[test]
fn starting_a_job_on_a_project_with_no_binding_is_refused_by_name() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));

    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            1,
            Request::Start {
                project: project().to_string(),
                kit: "Claude".to_owned(),
                work: "fix the build".to_owned(),
                title: String::new(),
            },
        ),
        Response::Refused(Refusal::ChannelMissing {
            project: "example".to_owned(),
        })
    );
    assert!(sim.first_turn().is_none(), "nothing ran: {:?}", sim.shape());
}

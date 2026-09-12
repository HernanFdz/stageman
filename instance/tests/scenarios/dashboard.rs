//! A person at the dashboard: every request is answered once whatever it
//! changed is on the disk, and refused with a reason the screen can show.

use stageman_core::{Agent, JobId, Outcome, Progress, ProjectId, Timestamp, Uuid, Waiting};
use stageman_instance::{Instance, Request, Response};
use stageman_wire::{ChannelDraft, Draft, Ending, Fitted, KitDraft, Refusal, Standing};

use crate::simulation::{Simulation, job, project, request, seed, watching, watching_a_channel};

/// Asks, performs, and lets the write land and the answer follow.
fn ask(sim: &mut Simulation, instance: &mut Instance, id: u64, asked: Request) -> Response {
    for effect in instance.step(request(id, asked)) {
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

fn a_draft(name: &str) -> Draft {
    Draft {
        name: name.to_owned(),
        repository: format!("https://example.invalid/{name}"),
        foreman: as_it_comes(),
        kits: vec![KitDraft {
            name: "Claude".to_owned(),
            description: "General-purpose.".to_owned(),
            fitted: as_it_comes(),
        }],
        credential: "ghp-not-a-real-token".to_owned(),
        channel: ChannelDraft::default(),
        variables: Vec::new(),
    }
}

/// Where in the trace a line first mentions something.
fn first(sim: &Simulation, what: &str) -> usize {
    let found = sim.trace().iter().position(|line| line.contains(what));
    assert!(
        found.is_some(),
        "nothing in the trace says {what}: {:#?}",
        sim.trace()
    );
    found.expect("asserted above")
}

fn count(sim: &Simulation, what: &str) -> usize {
    sim.trace()
        .iter()
        .filter(|line| line.contains(what))
        .count()
}

#[test]
fn the_instance_screen_counts_what_it_may_and_never_a_credential() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));
    let written = count(&sim, "-> Write");

    let Response::Instance(shown) = ask(&mut sim, &mut instance, 1, Request::Instance) else {
        panic!("the instance screen");
    };
    assert_eq!(shown.container_runtime, "/usr/local/bin/docker");
    assert_eq!(shown.agents, 1);
    assert_eq!(shown.projects.len(), 1);
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
/// has landed, and a second project on that channel is refused.
#[test]
fn creating_a_project_listens_on_its_channel_once_the_record_has_landed() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));
    assert!(sim.listening().is_empty(), "the one project has no channel");

    let mut draft = a_draft("burrow");
    draft.channel = ChannelDraft {
        address: "C0000000042".to_owned(),
        credential: "xoxb-not-a-real-token".to_owned(),
        listen_credential: "xapp-not-a-real-token".to_owned(),
    };
    let Response::Projects(shown) = ask(
        &mut sim,
        &mut instance,
        1,
        Request::Create {
            draft: draft.clone(),
        },
    ) else {
        panic!("the projects screen");
    };
    assert_eq!(shown.projects.len(), 2);
    let burrow = shown
        .projects
        .iter()
        .find(|project| project.name == "burrow")
        .expect("the new project");
    assert_eq!(burrow.channels, vec!["Slack".to_owned()]);
    assert_eq!(burrow.platforms, vec!["github".to_owned()]);
    let created = ProjectId::from_uuid(Uuid::parse_str(&burrow.id).expect("an identifier"));
    assert_eq!(sim.listening(), [created]);
    assert!(first(&sim, "-> Write") < first(&sim, "-> Listen"));
    assert!(first(&sim, "-> Listen") < first(&sim, "-> Respond"));
    assert!(sim.disk().expect("landed").projects.contains_key(&created));

    draft.name = "another".to_owned();
    assert_eq!(
        ask(&mut sim, &mut instance, 2, Request::Create { draft }),
        Response::Refused(Refusal::ChannelAlreadyBound {
            project: "burrow".to_owned()
        })
    );
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

/// Blank keeps, typed replaces, and what lands is what the screen was told.
#[test]
fn amending_keeps_a_credential_when_the_box_is_blank() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));
    let id = project().to_string();

    let mut blank = a_draft("example");
    blank.credential.clear();
    let Response::Projects(shown) = ask(
        &mut sim,
        &mut instance,
        1,
        Request::Amend {
            project: id.clone(),
            draft: blank.clone(),
        },
    ) else {
        panic!("the projects screen");
    };
    assert!(
        shown.projects[0].platforms.is_empty(),
        "it had none, it has none"
    );

    let mut typed = a_draft("renamed");
    typed.credential = "ghp-the-new-one".to_owned();
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
    assert_eq!(
        watched
            .credentials
            .get(&stageman_core::Platform::GitHub)
            .map(stageman_core::Secret::expose),
        Some("ghp-the-new-one")
    );

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
    let (name, held) = Simulation::ours(&stageman_job::container(job(1)));
    sim.container(&name, held);
    let (name, held) = Simulation::ours(&stageman_job::container(job(2)));
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
    assert!(!sim.exists(&stageman_job::container(job(1))));
    assert!(!sim.exists(&stageman_job::container(job(2))));
    assert!(!sim.exists(&stageman_foreman::container(project())));
    assert!(first(&sim, "-> Write") < first(&sim, "-> Discard"));
    assert!(sim.reclaims() >= 1);
    assert!(sim.disk().expect("landed").projects.is_empty());
}

/// A job started by hand is on the record before its turn runs, and its
/// turn runs only once the record has landed.
#[test]
fn starting_a_job_by_hand_runs_its_first_turn_once_the_record_has_landed() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
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
            at: Timestamp::UNIX_EPOCH,
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
    assert!(first(&sim, "-> Write") < first(&sim, "-> RunTurn"));
    let begun = JobId::from_uuid(Uuid::parse_str(&started.id).expect("an identifier"));
    assert!(sim.is_running(&stageman_job::container(begun)));

    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            2,
            Request::Start {
                project: id.clone(),
                kit: "gpt".to_owned(),
                work: "anything".to_owned(),
                at: Timestamp::UNIX_EPOCH,
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
                at: Timestamp::UNIX_EPOCH,
            },
        ),
        Response::Refused(Refusal::Incomplete {
            field: "work".to_owned()
        })
    );
    assert_eq!(instance.state().projects[&project()].jobs.len(), 1);
}

/// On a project with a channel, the thread is opened before the turn, and
/// the announcement is what is posted in it.
#[test]
fn a_job_started_by_hand_on_a_bound_project_opens_its_thread_first() {
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
            at: Timestamp::UNIX_EPOCH,
        },
    ) else {
        panic!("the project's screen");
    };
    assert_eq!(shown.jobs.len(), 1);
    assert!(first(&sim, "-> OpenThread") < first(&sim, "-> RunTurn"));
    assert_eq!(sim.posts().len(), 1);
    let started = JobId::from_uuid(Uuid::parse_str(&shown.jobs[0].id).expect("an identifier"));
    assert!(
        instance
            .state()
            .job(started)
            .and_then(|job| job.thread.clone())
            .is_some()
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
    let (name, held) = Simulation::ours(&stageman_job::container(job(1)));
    sim.container(&name, held);
    let (name, held) = Simulation::ours(&stageman_job::container(job(2)));
    sim.container(&name, held);
    let mut instance = sim.wake(seed(1));
    let id = project().to_string();

    for effect in instance.step(request(
        1,
        Request::Stop {
            project: id.clone(),
            job: job(1).to_string(),
        },
    )) {
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
    assert_eq!(count(&sim, "-> StopTurn"), 1);

    sim.run_until(&mut instance, 10);
    assert_eq!(
        instance.state().job(job(1)).map(|job| job.progress.clone()),
        Some(Progress::Idle(Waiting::Paused))
    );
    assert_eq!(
        sim.disk()
            .expect("landed")
            .job(job(1))
            .map(|job| job.progress.clone()),
        Some(Progress::Idle(Waiting::Paused))
    );
    assert!(
        sim.exists(&stageman_job::container(job(1))),
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
    assert_eq!(
        count(&sim, "-> StopTurn"),
        1,
        "nothing to stop in an idle job"
    );
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
    sim.holding(&watching(&[
        (job(1), Progress::Idle(Waiting::Asked)),
        (job(2), Progress::Working),
    ]));
    let (name, held) = Simulation::ours(&stageman_job::container(job(1)));
    sim.container(&name, held);
    let (name, held) = Simulation::ours(&stageman_job::container(job(2)));
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
    assert!(first(&sim, "-> Write") < first(&sim, "-> Discard"));
    assert!(!sim.exists(&stageman_job::container(job(1))));
    assert_eq!(sim.reclaims(), reclaimed + 1);
    assert_eq!(
        sim.disk()
            .expect("landed")
            .job(job(1))
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
        instance.state().job(job(1)).map(|job| job.progress.clone()),
        Some(Progress::Retired(Outcome::Done)),
        "a verdict is never overwritten"
    );
    assert_eq!(count(&sim, "-> Discard"), 2, "safe to press twice");
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
                at: Timestamp::UNIX_EPOCH,
            },
        );
        sim.run_until(&mut instance, 5_000);
        sim.trace().to_vec()
    };
    assert_eq!(run(), run());
}

//! Booting: everything the instance asks for before it can decide anything,
//! and the order it asks in.
//!
//! Constructed from a seed and the environment, the instance knows nothing
//! else. It asks whether each candidate for a container runtime answers,
//! in order; for its key, from the environment or from the file that keeps
//! it, minting one if there is none; for its own file, absent being a first
//! run; and for every container this project left behind, with the two
//! labels each carries. Only then does it wake — see
//! `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`.
//!
//! Every way a start used to refuse is an exit effect with its reason, and
//! the order that used to be kept by care — the runtime before the file,
//! because a machine with no runtime cannot run this at all, and the address
//! printed last, after the first write has landed — is a sequence a
//! scenario pins.
//!
//! Anything the world says meanwhile that is the application's own is kept
//! and handled once the instance is awake, in the order it arrived.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;

use rand::rngs::StdRng;
use rand::{Rng as _, SeedableRng as _};
use stageman_agent::{Command, Label, Target};
use stageman_core::{Agent, InstanceId, Key, State};
use stageman_vocabulary::{Bytes, Effect as Generic, EffectId, Environment, Finished, Seed};

use crate::tunnel::Domain;
use crate::vocabulary::{AppEvent, Container};
use crate::{Effect, Event, Running, file, paths};

/// Where the key came from, for the startup block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySource {
    /// The environment named it.
    Environment,
    /// The file that keeps it.
    Kept(PathBuf),
    /// Minted just now and written to the file that keeps it.
    Generated(PathBuf),
}

impl fmt::Display for KeySource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Environment => f.write_str(paths::KEY_VARIABLE),
            Self::Kept(path) => write!(f, "{}", path.display()),
            Self::Generated(path) => write!(f, "{} (generated just now)", path.display()),
        }
    }
}

/// Which question is in flight.
enum Phase {
    /// Whether the candidate answers.
    Runtime { candidate: usize, asked: EffectId },
    /// Whether the key file is there.
    Key { asked: EffectId },
    /// Whether the minted key landed.
    KeyWritten { asked: EffectId },
    /// Whether the instance's file is there.
    File { asked: EffectId },
    /// Which containers were left behind, and which are up.
    Listing {
        all: EffectId,
        up: EffectId,
        names: Option<Vec<String>>,
        running: Option<Vec<String>>,
    },
    /// What each container's labels say.
    Labels {
        pending: BTreeMap<EffectId, (String, Label)>,
        found: BTreeMap<String, (Option<InstanceId>, Option<Agent>)>,
        running: BTreeSet<String>,
    },
    /// Everything is known but where the dashboard is served.
    Ready { containers: Vec<Container> },
    /// A refusal was sent; nothing more will happen.
    Refused,
}

/// The instance before it is awake: what it has learned, and what it is
/// waiting to learn.
pub struct Boot {
    environment: Environment,
    rng: StdRng,
    next: u64,
    domain: Domain,
    /// Where the key is kept, if there is a home to keep it under. Needed
    /// only when the environment does not name one.
    key_file: Option<PathBuf>,
    instance_file: PathBuf,
    candidates: Vec<PathBuf>,
    phase: Phase,
    runtime: Option<PathBuf>,
    key: Option<(Key, KeySource)>,
    opened: Option<(State, Option<InstanceId>)>,
    /// Where the presentation server is, once the entry point says.
    presenting: Option<u16>,
    /// The address a person reaches this instance on, and the bind that
    /// answers for it.
    dashboard_asked: (String, Option<EffectId>),
    /// The port that bind took, once it has answered.
    dashboard: Option<u16>,
    /// The port the tools were asked for, and the bind that answers for it.
    tools_asked: (u16, Option<EffectId>),
    /// The port the tools are served on, once that bind has answered.
    tools: Option<u16>,
    /// The application's own events that arrived meanwhile, in order.
    waiting: Vec<AppEvent>,
    /// What there is to show while nothing is known.
    empty: State,
}

/// What a step of booting came to.
pub enum Booting {
    /// Still asking.
    Asking(Vec<Effect>),
    /// Awake, with what waking asked for and the events that waited.
    Awake(Box<Running>, Vec<Effect>, Vec<AppEvent>),
}

/// A failure and everything underneath it, as one line for whoever started
/// this process.
fn chain(failure: &dyn std::error::Error) -> String {
    let mut told = failure.to_string();
    let mut cause = std::error::Error::source(failure);
    while let Some(reason) = cause {
        told.push_str("\n  caused by: ");
        told.push_str(&reason.to_string());
        cause = reason.source();
    }
    told
}

impl Boot {
    /// Begins: the paths from the environment, and the first candidate for
    /// a runtime asked whether it answers.
    ///
    /// A machine with no home directory and nothing naming where the file
    /// is cannot keep an instance, and says so at once. A home is not needed
    /// for the key until the key is, since the environment may name it.
    ///
    /// The target is handed over rather than read, so that where this looks
    /// for a runtime and where it keeps its files are decided by what it was
    /// given: a start recorded on one platform replays on another.
    pub fn new(seed: Seed, environment: Environment, target: Target) -> (Self, Vec<Effect>) {
        let rng = StdRng::from_seed(seed);
        let domain = paths::domain(&environment);
        let tools_port = paths::tools_port(&environment);
        let dashboard_address = paths::dashboard_address(&environment);
        let key_file = paths::key_file(&environment, target).ok();
        let instance_file = match paths::instance_file(&environment, target) {
            Ok(instance_file) => instance_file,
            Err(why) => {
                let boot = Self {
                    environment,
                    rng,
                    next: 1,
                    domain,
                    key_file,
                    instance_file: PathBuf::new(),
                    candidates: Vec::new(),
                    phase: Phase::Refused,
                    runtime: None,
                    key: None,
                    opened: None,
                    presenting: None,
                    dashboard_asked: (String::new(), None),
                    dashboard: Some(0),
                    tools_asked: (0, None),
                    tools: Some(0),
                    waiting: Vec::new(),
                    empty: State::default(),
                };
                return (
                    boot,
                    vec![Generic::Exit {
                        message: why.to_string(),
                    }],
                );
            }
        };
        let mut boot = Self {
            environment,
            rng,
            next: 1,
            domain,
            key_file,
            instance_file,
            candidates: stageman_agent::candidates(target)
                .iter()
                .map(PathBuf::from)
                .collect(),
            phase: Phase::Refused,
            runtime: None,
            key: None,
            opened: None,
            presenting: None,
            dashboard_asked: (dashboard_address, None),
            dashboard: None,
            tools_asked: (tools_port, None),
            tools: None,
            waiting: Vec::new(),
            empty: State::default(),
        };
        let mut effects = boot.try_candidate(0);
        effects.extend(boot.take_the_addresses());
        (boot, effects)
    }

    /// Asks for the two addresses this instance answers on.
    ///
    /// The dashboard's is where a person types, and what arrives there is
    /// routed by the name it was asked for: a job's tunnel to that job, and
    /// everything else to the presentation server.
    ///
    /// The tools' is every interface, which is not a preference: it is the
    /// only address a container can reach on every platform, measured on
    /// both runtimes — see
    /// `docs/decisions/0033-the-job-endpoint-listens-beyond-loopback.md`.
    ///
    /// Both while booting rather than after, so that nothing is served
    /// before there is anything to answer with, and no turn begins before
    /// the port a container would be told about is known.
    fn take_the_addresses(&mut self) -> Vec<Effect> {
        let dashboard = self.effect_id();
        self.dashboard_asked.1 = Some(dashboard);
        let tools = self.effect_id();
        self.tools_asked.1 = Some(tools);
        vec![
            Generic::Bind {
                id: dashboard,
                address: self.dashboard_asked.0.clone(),
            },
            Generic::Bind {
                id: tools,
                address: format!("0.0.0.0:{}", self.tools_asked.0),
            },
        ]
    }

    /// What there is to show while nothing is known.
    pub const fn state(&self) -> &State {
        &self.empty
    }

    /// A name for where booting has got to, for a snapshot.
    pub const fn phase_name(&self) -> &'static str {
        match self.phase {
            Phase::Runtime { .. } => "runtime",
            Phase::Key { .. } => "key",
            Phase::KeyWritten { .. } => "key written",
            Phase::File { .. } => "file",
            Phase::Listing { .. } => "listing",
            Phase::Labels { .. } => "labels",
            Phase::Ready { .. } => "ready",
            Phase::Refused => "refused",
        }
    }

    const fn effect_id(&mut self) -> EffectId {
        let id = EffectId(self.next);
        // An identifier only has to be unique among the effects still
        // waiting to be answered, and booting waits on a handful.
        self.next = self.next.wrapping_add(1); // CLAMP-OK: the cycle is `EffectId`'s contract.
        id
    }

    /// The environment a runtime command is given: this process's own, less
    /// this project's variables, which are its and not the runtime's.
    fn runtime_environment(&self) -> Environment {
        self.environment
            .iter()
            .filter(|(name, _)| !name.starts_with("STAGEMAN_"))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect()
    }

    /// Asks a runtime command of the runtime that answered.
    fn run(&mut self, program: PathBuf, command: &Command) -> (EffectId, Effect) {
        let id = self.effect_id();
        let effect = Generic::Run {
            id,
            program,
            arguments: command.arguments(),
            environment: self.runtime_environment(),
            stdin: None,
        };
        (id, effect)
    }

    /// Asks the next candidate whether it answers, or refuses when there is
    /// none left.
    fn try_candidate(&mut self, candidate: usize) -> Vec<Effect> {
        let Some(program) = self.candidates.get(candidate).cloned() else {
            return self.no_runtime();
        };
        let (asked, effect) = self.run(program, &Command::Version);
        self.phase = Phase::Runtime { candidate, asked };
        vec![effect]
    }

    /// Refuses, because the list of candidates is exhausted and none of them
    /// answered.
    fn no_runtime(&mut self) -> Vec<Effect> {
        let looked = self
            .candidates
            .iter()
            .map(|candidate| format!("    {}", candidate.display()))
            .collect::<Vec<_>>()
            .join("\n");
        self.refuse(format!(
            "no container runtime found.\n  Every agent runs in a container, including the \
             one a foreman thinks with,\n  so nothing here can run without one. Install Docker \
             or Podman.\n  Looked in:\n{looked}"
        ))
    }

    /// Stops here, with the reason as the process's last word.
    fn refuse(&mut self, message: String) -> Vec<Effect> {
        self.phase = Phase::Refused;
        vec![Generic::Exit { message }]
    }

    /// Handles one event: an answer to what was asked, or something of the
    /// application's own to keep for later.
    pub fn step(&mut self, event: Event) -> Booting {
        match event {
            Event::App(AppEvent::Presenting { port }) => {
                self.presenting = Some(port);
                self.maybe_awake(Vec::new())
            }
            Event::App(event) => {
                self.waiting.push(event);
                Booting::Asking(Vec::new())
            }
            Event::Ran { id, finished } => self.ran(id, finished),
            Event::Read { id, contents } => self.read(id, contents),
            Event::Written { id, outcome } => self.written(id, outcome),
            // Booting asks for no timer and no listener, so an answer to
            // either is somebody else's and is ignored rather than acted on.
            Event::Bound { id, outcome } if Some(id) == self.dashboard_asked.1 => {
                match outcome {
                    Ok(taken) => self.dashboard = Some(taken),
                    // Nothing else works without it: a dashboard nobody can
                    // reach is the door every repair happens behind.
                    Err(why) => {
                        return Booting::Asking(self.refuse(format!(
                            "the dashboard cannot listen on {}\n  caused by: {why}",
                            self.dashboard_asked.0
                        )));
                    }
                }
                self.maybe_awake(Vec::new())
            }
            Event::Bound { id, outcome } if Some(id) == self.tools_asked.1 => {
                let asked = self.tools_asked.0;
                self.tools = Some(match outcome {
                    Ok(taken) => taken,
                    Err(why) => {
                        // Said rather than refused over: what is lost is a
                        // foreman's ability to start a job, and an instance
                        // that will not start puts the repair behind the door
                        // it just locked — `docs/conventions.md` §3.
                        tracing::error!(
                            port = asked,
                            %why,
                            "the tools could not be served, so no foreman can ask for a job"
                        );
                        asked
                    }
                });
                self.maybe_awake(Vec::new())
            }
            // Booting asks for no timer and no request, so an answer to
            // either is somebody else's and is ignored rather than acted on.
            Event::Woke { .. }
            | Event::Bound { .. }
            | Event::Arrived { .. }
            | Event::Body { .. } => Booting::Asking(Vec::new()),
        }
    }

    /// A program finished: a runtime candidate, or a runtime command.
    fn ran(&mut self, id: EffectId, finished: Finished) -> Booting {
        match std::mem::replace(&mut self.phase, Phase::Refused) {
            Phase::Runtime { candidate, asked } if asked == id => {
                Booting::Asking(self.runtime_answered(candidate, finished))
            }
            Phase::Listing {
                all,
                up,
                names,
                running,
            } if all == id || up == id => self.listed((all, up), (names, running), id, finished),
            Phase::Labels {
                pending,
                found,
                running,
            } => self.labelled(pending, found, running, id, finished),
            other => {
                tracing::warn!("a program finished that booting did not ask about; ignored");
                self.phase = other;
                Booting::Asking(Vec::new())
            }
        }
    }

    /// A runtime candidate answered its version check, or could not be run.
    fn runtime_answered(&mut self, candidate: usize, finished: Finished) -> Vec<Effect> {
        match finished {
            Finished::Exited {
                status: Some(0), ..
            } => {
                self.runtime = self.candidates.get(candidate).cloned();
                self.ask_key()
            }
            Finished::Exited { stderr, .. } => {
                let program = self
                    .candidates
                    .get(candidate)
                    .map_or_else(String::new, |program| program.display().to_string());
                let complaint = stderr.as_text().unwrap_or("").trim().to_owned();
                self.refuse(format!(
                    "the container runtime is not usable\n  caused by: {program} does not \
                     answer: {complaint}"
                ))
            }
            // Not there, or not startable: either way, the next one. An
            // index that cannot be advanced names no candidate, which is the
            // same answer as having reached the end of the list.
            Finished::NotFound | Finished::Failed(_) => match candidate.checked_add(1) {
                Some(next) => self.try_candidate(next),
                None => self.no_runtime(),
            },
        }
    }

    /// One of the two listings answered. Once both have, every container
    /// named is asked for its labels.
    fn listed(
        &mut self,
        (all, up): (EffectId, EffectId),
        (mut names, mut running): (Option<Vec<String>>, Option<Vec<String>>),
        id: EffectId,
        finished: Finished,
    ) -> Booting {
        let listed =
            match finished {
                Finished::Exited {
                    status: Some(0),
                    stdout,
                    ..
                } => stageman_agent::names(stdout.as_text().unwrap_or("")),
                Finished::Exited { stderr, .. } => {
                    let complaint = stderr.as_text().unwrap_or("").trim().to_owned();
                    return Booting::Asking(self.refuse(format!(
                        "what the last run left behind could not be established\n  caused by: the \
                     runtime refused to list containers: {complaint}"
                    )));
                }
                Finished::NotFound | Finished::Failed(_) => {
                    return Booting::Asking(self.refuse(
                    "what the last run left behind could not be established\n  caused by: the \
                     runtime could not be run"
                        .to_owned(),
                ));
                }
            };
        if all == id {
            names = Some(listed);
        } else {
            running = Some(listed);
        }
        match (names, running) {
            (Some(names), Some(running)) => {
                let asked = self.ask_labels(names, running);
                self.maybe_awake(asked)
            }
            (names, running) => {
                self.phase = Phase::Listing {
                    all,
                    up,
                    names,
                    running,
                };
                Booting::Asking(Vec::new())
            }
        }
    }

    /// A container's label was read, or could not be. Once every label has
    /// been, the containers are known.
    fn labelled(
        &mut self,
        mut pending: BTreeMap<EffectId, (String, Label)>,
        mut found: BTreeMap<String, (Option<InstanceId>, Option<Agent>)>,
        running: BTreeSet<String>,
        id: EffectId,
        finished: Finished,
    ) -> Booting {
        let Some((name, label)) = pending.remove(&id) else {
            tracing::warn!("a program finished that booting did not ask about; ignored");
            self.phase = Phase::Labels {
                pending,
                found,
                running,
            };
            return Booting::Asking(Vec::new());
        };
        let text = if let Finished::Exited {
            status: Some(0),
            stdout,
            ..
        } = finished
        {
            stdout.as_text().unwrap_or("").to_owned()
        } else {
            tracing::warn!(container = %name, "could not read a container's label");
            String::new()
        };
        let entry = found.entry(name).or_insert((None, None));
        match label {
            Label::Instance => entry.0 = stageman_agent::minted(&text),
            Label::Agent => entry.1 = stageman_agent::labelled(text.trim()),
        }
        if pending.is_empty() {
            let containers = found
                .into_iter()
                .map(|(name, (instance, agent))| Container {
                    running: running.contains(&name),
                    name,
                    instance,
                    agent,
                })
                .collect();
            self.phase = Phase::Ready { containers };
            self.maybe_awake(Vec::new())
        } else {
            self.phase = Phase::Labels {
                pending,
                found,
                running,
            };
            Booting::Asking(Vec::new())
        }
    }

    /// Asks for the key: from the environment if it names one, else from the
    /// file that keeps it.
    fn ask_key(&mut self) -> Vec<Effect> {
        if let Some(named) = paths::told(&self.environment, paths::KEY_VARIABLE) {
            return match Key::from_base64(&named) {
                Ok(key) => {
                    self.key = Some((key, KeySource::Environment));
                    self.ask_file()
                }
                Err(why) => self.refuse(format!(
                    "the instance key is not usable\n  caused by: {why}"
                )),
            };
        }
        let Some(key_file) = self.key_file.clone() else {
            return self.refuse(paths::NoHome.to_string());
        };
        let asked = self.effect_id();
        self.phase = Phase::Key { asked };
        vec![Generic::Read {
            id: asked,
            path: key_file,
        }]
    }

    /// Asks for the instance's own file.
    fn ask_file(&mut self) -> Vec<Effect> {
        let asked = self.effect_id();
        self.phase = Phase::File { asked };
        vec![Generic::Read {
            id: asked,
            path: self.instance_file.clone(),
        }]
    }

    /// A file was read: the key's, or the instance's own.
    fn read(&mut self, id: EffectId, contents: Result<Option<Bytes>, String>) -> Booting {
        match std::mem::replace(&mut self.phase, Phase::Refused) {
            Phase::Key { asked } if asked == id => {
                let key_file = self.key_file.clone().unwrap_or_default();
                let effects = match contents {
                    Ok(Some(bytes)) => match Key::from_base64(bytes.as_text().unwrap_or("").trim())
                    {
                        Ok(key) => {
                            self.key = Some((key, KeySource::Kept(key_file)));
                            self.ask_file()
                        }
                        Err(why) => self.refuse(format!(
                            "the instance key is not usable\n  caused by: {why}"
                        )),
                    },
                    Ok(None) => {
                        // Minted here, from the one generator, and written
                        // before anything depends on it. Readable by this
                        // user alone, where the platform can say so.
                        let mut material = [0_u8; 32];
                        self.rng.fill_bytes(&mut material);
                        let key = Key::new(material);
                        let asked = self.effect_id();
                        self.phase = Phase::KeyWritten { asked };
                        let written = Generic::Write {
                            id: asked,
                            path: key_file.clone(),
                            bytes: format!("{}\n", key.to_base64()).into(),
                            private: true,
                        };
                        self.key = Some((key, KeySource::Generated(key_file)));
                        vec![written]
                    }
                    Err(why) => self.refuse(format!(
                        "the instance key at {} could not be read or written\n  caused by: {why}",
                        key_file.display()
                    )),
                };
                Booting::Asking(effects)
            }
            Phase::File { asked } if asked == id => {
                let effects = match contents {
                    Ok(found) => {
                        let Some((key, _)) = &self.key else {
                            return Booting::Asking(
                                self.refuse("no key to open the instance with".to_owned()),
                            );
                        };
                        match file::opened(found.as_ref().map(Bytes::as_slice), key) {
                            Ok(opened) => {
                                for project in unheard(&opened.0) {
                                    tracing::warn!(
                                        %project,
                                        "a channel is bound with no credential to listen with, so \
                                         nothing it says will be heard — which is indistinguishable \
                                         from nobody saying anything"
                                    );
                                }
                                self.opened = Some(opened);
                                self.ask_listing()
                            }
                            Err(why) => self.refuse(format!(
                                "the instance at {} could not be opened\n  caused by: {}",
                                self.instance_file.display(),
                                chain(&why)
                            )),
                        }
                    }
                    Err(why) => self.refuse(format!(
                        "the instance at {} could not be read\n  caused by: {why}",
                        self.instance_file.display()
                    )),
                };
                Booting::Asking(effects)
            }
            other => {
                tracing::warn!("a file was read that booting did not ask about; ignored");
                self.phase = other;
                Booting::Asking(Vec::new())
            }
        }
    }

    /// The minted key landed, or did not.
    fn written(&mut self, id: EffectId, outcome: Result<(), String>) -> Booting {
        match std::mem::replace(&mut self.phase, Phase::Refused) {
            Phase::KeyWritten { asked } if asked == id => {
                let key_file = self.key_file.clone().unwrap_or_default();
                let effects = match outcome {
                    Ok(()) => self.ask_file(),
                    Err(why) => self.refuse(format!(
                        "the instance key at {} could not be read or written\n  caused by: {why}",
                        key_file.display()
                    )),
                };
                Booting::Asking(effects)
            }
            other => {
                tracing::warn!("a write landed that booting did not ask about; ignored");
                self.phase = other;
                Booting::Asking(Vec::new())
            }
        }
    }

    /// Asks which containers this project left behind, and which are up.
    fn ask_listing(&mut self) -> Vec<Effect> {
        let Some(program) = self.runtime.clone() else {
            return self.refuse("no runtime to list containers with".to_owned());
        };
        let (all, listing) = self.run(
            program.clone(),
            &Command::Containers {
                running_only: false,
            },
        );
        let (up, running) = self.run(program, &Command::Containers { running_only: true });
        self.phase = Phase::Listing {
            all,
            up,
            names: None,
            running: None,
        };
        vec![listing, running]
    }

    /// Asks what each container's labels say, all at once.
    fn ask_labels(&mut self, names: Vec<String>, running: Vec<String>) -> Vec<Effect> {
        let Some(program) = self.runtime.clone() else {
            return self.refuse("no runtime to inspect containers with".to_owned());
        };
        let mut pending = BTreeMap::new();
        let mut effects = Vec::new();
        let mut found = BTreeMap::new();
        for name in names {
            found.insert(name.clone(), (None, None));
            for label in [Label::Instance, Label::Agent] {
                let (id, effect) = self.run(
                    program.clone(),
                    &Command::Label {
                        name: name.clone(),
                        label,
                    },
                );
                pending.insert(id, (name.clone(), label));
                effects.push(effect);
            }
        }
        let running: BTreeSet<String> = running.into_iter().collect();
        if pending.is_empty() {
            self.phase = Phase::Ready {
                containers: Vec::new(),
            };
            return effects;
        }
        self.phase = Phase::Labels {
            pending,
            found,
            running,
        };
        effects
    }

    /// Wakes once everything is known, including where the dashboard is.
    fn maybe_awake(&mut self, effects: Vec<Effect>) -> Booting {
        if self.presenting.is_none() || self.dashboard.is_none() || self.tools.is_none() {
            return Booting::Asking(effects);
        }
        let containers = match std::mem::replace(&mut self.phase, Phase::Refused) {
            Phase::Ready { containers } => containers,
            other => {
                self.phase = other;
                return Booting::Asking(effects);
            }
        };
        let (
            Some(runtime),
            Some((key, source)),
            Some((state, named)),
            Some(port),
            Some(presenting),
        ) = (
            self.runtime.take(),
            self.key.take(),
            self.opened.take(),
            self.dashboard.take(),
            self.presenting.take(),
        )
        else {
            return Booting::Asking(self.refuse("booting lost a fact it had".to_owned()));
        };
        // What a person types, with the port that was actually taken: asking
        // for zero is asking for whichever is free, and what is announced
        // has to be somewhere they can go.
        let address = self.dashboard_asked.0.rsplit_once(':').map_or_else(
            || self.dashboard_asked.0.clone(),
            |(host, _)| format!("{host}:{port}"),
        );
        let waiting = std::mem::take(&mut self.waiting);
        let mut running = Running::woken(crate::Facts {
            state,
            named,
            key,
            source,
            rng: std::mem::replace(&mut self.rng, StdRng::from_seed([0; 32])),
            next: self.next,
            domain: self.domain.clone(),
            path: self.instance_file.clone(),
            runtime,
            runtime_environment: self.runtime_environment(),
            tools: self.tools.unwrap_or(self.tools_asked.0),
            tools_listener: self.tools_asked.1,
            dashboard_listener: self.dashboard_asked.1,
            presenting,
            address,
            port,
        });
        let mut asked = effects;
        asked.extend(running.waking_up(&containers));
        Booting::Awake(Box::new(running), asked, waiting)
    }
}

/// Every project whose channels this instance cannot hear replies on.
///
/// By name rather than by count: an operator told that two of three projects
/// are listening still has to work out which one is not. A binding with no
/// credential to listen with is not an error and produces no warning of its
/// own — it looks exactly like a platform that has sent nothing — so this is
/// the only thing that tells the two apart.
pub fn unheard(state: &State) -> Vec<String> {
    state
        .projects
        .values()
        .filter(|project| {
            project
                .channels
                .values()
                .any(|bound| bound.listen_credential.is_none())
        })
        .map(|project| project.name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::unheard;
    use stageman_core::{Agent, Channel, ChannelConfig, Project, ProjectId, Secret, State, Uuid};
    use std::collections::BTreeMap;

    fn watching(name: &str, listens: bool) -> Project {
        Project {
            name: name.to_owned(),
            repository: "https://example.invalid/repo".to_owned(),
            foreman_kit: stageman_core::Kit::defaults(Agent::Claude),
            kits: BTreeMap::from([(
                stageman_core::KitName::new("Claude").expect("a name"),
                stageman_core::KitConfig::defaults(Agent::Claude),
            )]),
            credentials: BTreeMap::new(),
            channels: BTreeMap::from([(
                Channel::Slack,
                ChannelConfig {
                    address: format!("C-{name}"),
                    credential: Secret::new("xoxb-token".to_owned()),
                    listen_credential: listens.then(|| Secret::new("xapp-token".to_owned())),
                },
            )]),
            jobs: BTreeMap::new(),
            variables: BTreeMap::new(),
            attending: stageman_core::Attending::default(),
        }
    }

    #[test]
    fn only_a_project_that_cannot_be_heard_is_named() {
        let mut state = State::default();
        assert!(unheard(&state).is_empty());

        state.projects.insert(
            ProjectId::from_uuid(Uuid::from_u128(1)),
            watching("heard", true),
        );
        assert!(
            unheard(&state).is_empty(),
            "a listening channel is not a problem"
        );

        state.projects.insert(
            ProjectId::from_uuid(Uuid::from_u128(2)),
            watching("deaf", false),
        );
        assert_eq!(unheard(&state), vec!["deaf".to_owned()]);
    }
}

//! Everything the instance holds, as one value a scenario compares and a
//! reviewer reads.
//!
//! Credentials in the clear, deliberately: this is test support and the
//! flight-recorder question in `docs/open-questions.md`, and every scenario
//! file holds fake ones. The domain's own types refuse to serialise a secret,
//! so the kept state is walked here rather than derived, which is also the
//! one place a reviewer can read what an instance knows in full.

use serde_json::{Map, Value, json};
use stageman_core::State;

use crate::Running;

/// A map with string keys, from anything whose keys print.
fn keyed<K: ToString, V: Into<Value>>(pairs: impl IntoIterator<Item = (K, V)>) -> Value {
    Value::Object(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_string(), value.into()))
            .collect::<Map<String, Value>>(),
    )
}

/// A value, or null where it will not serialise, which nothing here should
/// do.
fn value<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// The kept state, credentials exposed.
#[must_use]
pub fn exposed(state: &State) -> Value {
    json!({
        "agents": keyed(state.agents.iter().map(|(agent, config)| {
            (format!("{agent:?}"), json!({ "auth_token": config.auth_token.expose() }))
        })),
        "projects": keyed(state.projects.iter().map(|(id, project)| {
            (id, json!({
                "name": project.name,
                "repository": project.repository,
                "foreman_kit": value(&project.foreman_kit),
                "kits": keyed(project.kits.iter().map(|(name, offered)| {
                    (name, json!({ "description": offered.description, "kit": value(&offered.kit) }))
                })),
                "credentials": keyed(project.credentials.iter().map(|(platform, secret)| {
                    (format!("{platform:?}"), secret.expose().to_owned())
                })),
                "channels": keyed(project.channels.iter().map(|(channel, bound)| {
                    (format!("{channel:?}"), json!({
                        "address": bound.address,
                        "credential": bound.credential.expose(),
                        "listen_credential": bound.listen_credential.as_ref().map(|it| it.expose().to_owned()),
                    }))
                })),
                "variables": keyed(project.variables.iter().map(|(name, secret)| {
                    (name, secret.expose().to_owned())
                })),
                "jobs": value(&project.jobs),
                "attending": value(&project.attending),
            }))
        })),
    })
}

/// Everything an awake instance holds: what goes to the disk, and what only
/// this process knows.
#[must_use]
pub fn of(running: &Running) -> Value {
    json!({
        "kept": exposed(&running.state),
        "held": {
            "id": running.id.to_string(),
            "key": running.key.to_base64(),
            "key_source": running.source.to_string(),
            "path": running.path.display().to_string(),
            "domain": running.domain.to_string(),
            "serving": running.serving,
            "address": running.address,
            "runtime": running.runtime.display().to_string(),
            "runtime_environment": value(&running.runtime_environment),
            "next": running.next,
            "turns": value(&running.turns.iter().collect::<Vec<_>>()),
            "warrants": value(&running.warrants),
            "interrupted": value(&running.interrupted),
            "asking": value(&running.asking.iter().collect::<Vec<_>>()),
            "tunnels": value(&running.tunnels.iter().collect::<Vec<_>>()),
            "routing": value(&running.routing.iter().collect::<Vec<_>>()),
            "deferred": value(&running.deferred),
            "timers": value(&running.timers),
            "asked": value(&running.asked.iter().collect::<Vec<_>>()),
            "listing": value(&running.listing.iter().collect::<Vec<_>>()),
            "dirty": running.dirty,
            "staged": value(&running.staged),
            "announced": running.announced,
            "swept": value(&running.swept),
        },
    })
}

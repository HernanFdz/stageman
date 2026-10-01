//! The purses an instance holds, by provider, and the agents it can run.
//!
//! First of the configuring screens, because nothing else can be configured
//! until a purse is held: a project names an agent for its foreman and a
//! non-empty set of kits its jobs may run on, and every one of those
//! charges a purse — see
//! `docs/decisions/0086-a-kit-charges-a-purse-at-a-provider.md`. An instance
//! with no purse is not broken, it is new — and this is the screen that
//! ends that.
//!
//! The purses come first because they are what is configured. An agent is
//! ready or not by what is held, and nothing is set on an agent itself.
//!
//! **A credential travels one way.** It is sent here and never sent back:
//! nothing on this page carries one, and [`PurseView`] has nowhere to put
//! one, which is the invariant in `docs/architecture.md` §2 expressed as a
//! type rather than as care.

use dioxus::prelude::*;
#[cfg(feature = "server")]
use stageman_instance::{Request, Response};

use super::error::{DashboardError, DashboardResult};
use super::live::{Live, Reading, use_reading};
use crate::ui::{
    BESIDE, Badge, BadgeTone, Button, ButtonVariant, Card, EmptyState, Guide, Icon, Mark,
    SecretBox, Skeleton, Tooltip,
};

pub use stageman_wire::{Agent, Agents, ProviderView, PurseView};

/// Every purse, held or not, by provider, and every agent.
///
/// The whole set rather than what is held: this screen's job is to let
/// somebody hold a purse, and a list that hides what is missing cannot.
///
/// # Errors
///
/// Fails if this process is not operating an instance.
#[get("/api/agents")]
pub async fn agents() -> DashboardResult<Agents> {
    match super::ask(Request::Agents).await? {
        Response::Agents(listing) => Ok(listing),
        other => Err(super::unexpected(&other)),
    }
}

/// Holds a purse, or replaces the credential it holds.
///
/// Replacing rather than refusing when one is already held, because
/// rotating a credential is the ordinary reason to come back to this screen
/// and a separate verb for it would be ceremony; a replaced credential
/// reaches every agent charging the purse at its next turn.
///
/// # Errors
///
/// Fails if the purse is not one this build knows, if the credential is
/// empty, or if it has the shape of the other box's.
#[post("/api/purses/hold")]
pub async fn hold(purse: String, credential: String) -> DashboardResult<Agents> {
    match super::ask(Request::HoldPurse { purse, credential }).await? {
        Response::Agents(listing) => Ok(listing),
        other => Err(super::unexpected(&other)),
    }
}

/// Forgets a purse, if nothing charges it.
///
/// # Errors
///
/// Fails if the purse is not one this build knows, or if a foreman, an
/// offered kit or an unfinished job still charges it.
#[post("/api/purses/forget")]
pub async fn forget(purse: String) -> DashboardResult<Agents> {
    match super::ask(Request::ForgetPurse { purse }).await? {
        Response::Agents(listing) => Ok(listing),
        other => Err(super::unexpected(&other)),
    }
}

/// The agents screen: one card per provider with its purses, then the
/// agents and whether each is ready.
#[component]
pub fn AgentsView() -> Element {
    let live = use_context::<Live>();
    let reading = use_reading(live, agents)?;
    let mut failure = use_signal(|| None::<DashboardError>);

    rsx! {
        div { class: "flex flex-col gap-4",
            if let Some(reason) = failure() {
                Card { title: "That did not work",
                    p { class: "text-sm text-failed", "{reason}" }
                }
            }
            match reading {
                Reading::Read(mut listing) => {
                    let shown = listing();
                    let ready = shown.agents.iter().filter(|agent| agent.ready).count();
                    rsx! {
                        for provider in shown.providers.iter() {
                            {
                                let held = provider.purses.iter().filter(|purse| purse.held).count();
                                let total = provider.purses.len();
                                rsx! {
                                    Card {
                                        key: "{provider.id}",
                                        title: provider.name.clone(),
                                        note: "What a kit's work is charged to: a key, metered per token, or a \
                                               subscription, flat per month. Paste each into its own box.",
                                        badge: rsx! { Badge { "{held} of {total} held" } },
                                        ul { class: "divide-y divide-border",
                                            for purse in provider.purses.iter() {
                                                li { key: "{purse.id}",
                                                    PurseRow {
                                                        provider: provider.id.clone(),
                                                        purse: purse.clone(),
                                                        onchanged: move |outcome: DashboardResult<Agents>| {
                                                            match outcome {
                                                                Ok(fresh) => {
                                                                    failure.set(None);
                                                                    listing.set(fresh);
                                                                }
                                                                Err(reason) => failure.set(Some(reason)),
                                                            }
                                                        },
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Card {
                            title: "Agents",
                            note: "Each is ready once a purse it can charge is held; nothing is set on an agent itself.",
                            badge: rsx! { Badge { "{ready} of {shown.agents.len()} ready" } },
                            if shown.agents.is_empty() {
                                EmptyState {
                                    title: "This build can run no agents at all.",
                                    note: "The set is compiled in, so this is a build problem rather \
                                           than something to configure.",
                                }
                            } else {
                                ul { class: "divide-y divide-border",
                                    for agent in shown.agents.iter() {
                                        li { key: "{agent.id}",
                                            AgentRow { agent: agent.clone(), providers: shown.providers.clone() }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Reading::Failed(reason) => rsx! {
                    Card { title: "The agents could not be read",
                        p { class: "text-sm text-failed", "{reason}" }
                    }
                },
                Reading::NotYet => rsx! { Skeleton {} },
            }
        }
    }
}

/// One purse: what it is, whether it is held and by what it is charged,
/// and whatever it is currently possible to do to it.
///
/// A box to paste into while nothing is held or a replacement is wanted,
/// and otherwise the two things a held purse offers — replacing and
/// forgetting — with forgetting greyed and explained while something charges
/// it. What charges it is a hover away on the row rather than written out,
/// because a purse a project has been running on for a while is charged by
/// every unfinished job, and a list that long is a wall. The guide to where
/// the credential is minted sits at the end of the line, whatever the
/// state. The box's placeholder is what a credential of its kind begins
/// with, so the box teaches the shape before a refusal does, and Save is
/// offered only once there is something to save.
#[component]
fn PurseRow(
    provider: String,
    purse: PurseView,
    onchanged: EventHandler<DashboardResult<Agents>>,
) -> Element {
    let mut credential = use_signal(String::new);
    let mut replacing = use_signal(|| false);
    let in_use = !purse.charged_by.is_empty();
    let charged_by = format!("Charged by {}", purse.charged_by.join(", "));
    let forgetting_says = if in_use {
        format!("{charged_by}, so it cannot be forgotten")
    } else {
        "Forget this purse".to_owned()
    };
    let charged_by_agents = purse.agents.join(", ");
    let identifier = purse.id.clone();
    let pasting = !purse.held || replacing();

    rsx! {
        div { class: "flex flex-col gap-2 py-3 first:pt-0 last:pb-0",
            div { class: "flex items-center gap-3",
                Mark { agent: provider.clone(), size: 14 }
                span { class: "shrink-0 text-sm font-medium", "{purse.name}" }
                if purse.held {
                    Badge { tone: BadgeTone::Idle, "held" }
                } else {
                    Badge { "not held" }
                }
                if in_use {
                    Tooltip { text: charged_by, wrap: true,
                        Badge { "in use" }
                    }
                }
            }
            p { class: "max-w-prose text-xs text-muted-foreground",
                "{purse.note} Charged by {charged_by_agents}."
            }
            div { class: "flex flex-wrap items-center gap-2",
                if pasting {
                    SecretBox {
                        class: "max-w-sm text-xs",
                        placeholder: purse.example.clone(),
                        aria_label: "The {purse.name}",
                        value: credential(),
                        oninput: move |event: FormEvent| credential.set(event.value()),
                    }
                    Button {
                        disabled: credential().trim().is_empty(),
                        onclick: move |_| {
                            let identifier = identifier.clone();
                            let supplied = credential();
                                async move {
                                    let outcome = hold(identifier, supplied).await;
                                    if outcome.is_ok() {
                                        credential.set(String::new());
                                        replacing.set(false);
                                    }
                                    onchanged.call(outcome);
                                }
                        },
                        "Save"
                    }
                    if purse.held {
                        Button {
                            variant: ButtonVariant::Secondary,
                            onclick: move |_| {
                                credential.set(String::new());
                                replacing.set(false);
                            },
                            "Keep the old one"
                        }
                    }
                } else {
                    Button {
                        variant: ButtonVariant::Secondary,
                        onclick: move |_| replacing.set(true),
                        "Replace"
                    }
                    // Muted until hovered, like every control that discards,
                    // and greyed with the reason while something charges it.
                    Tooltip { text: forgetting_says.clone(), wrap: in_use,
                        Button {
                            variant: ButtonVariant::Secondary,
                            class: BESIDE,
                            disabled: in_use,
                            aria_label: "{forgetting_says}",
                            onclick: move |_| {
                                let identifier = identifier.clone();
                                async move { onchanged.call(forget(identifier).await) }
                            },
                            {Icon::Remove.draw(16)}
                        }
                    }
                }
                span { class: "ml-auto",
                    Guide {
                        mark: provider,
                        label: purse.minting,
                        says: purse.guidance,
                        link: purse.guide,
                    }
                }
            }
        }
    }
}

/// One agent: what it is good for, whether it is ready, and which purses
/// would make it so. Read-only, since nothing is set on an agent itself.
#[component]
fn AgentRow(agent: Agent, providers: Vec<ProviderView>) -> Element {
    let charging = charges_of(&agent, &providers).join("; ");

    rsx! {
        div { class: "flex flex-col gap-1 py-3 first:pt-0 last:pb-0",
            div { class: "flex items-center gap-3",
                Mark { agent: agent.id.clone(), size: 14 }
                span { class: "text-sm font-medium", "{agent.name}" }
                if agent.ready {
                    Badge { tone: BadgeTone::Idle, "ready" }
                } else {
                    Badge { "needs a purse" }
                }
            }
            p { class: "max-w-prose text-xs text-muted-foreground", "{agent.description}" }
            p { class: "max-w-prose text-xs text-muted-foreground", "Charges {charging}." }
        }
    }
}

/// What an agent can charge, provider by provider, in the words the
/// providers' cards use for each purse.
///
/// Pure, so the sentence a row says can be tested without a browser.
fn charges_of(agent: &Agent, providers: &[ProviderView]) -> Vec<String> {
    providers
        .iter()
        .filter_map(|provider| {
            let names: Vec<&str> = provider
                .purses
                .iter()
                .filter(|purse| agent.purses.contains(&purse.id))
                .map(|purse| purse.name.as_str())
                .collect();
            (!names.is_empty()).then(|| format!("{}: {}", provider.name, names.join(" or ")))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Agent, ProviderView, PurseView, charges_of};

    fn purse(id: &str, name: &str) -> PurseView {
        PurseView {
            id: id.to_owned(),
            name: name.to_owned(),
            note: String::new(),
            example: String::new(),
            guide: String::new(),
            minting: String::new(),
            guidance: String::new(),
            held: false,
            charged_by: Vec::new(),
            agents: Vec::new(),
        }
    }

    /// An agent's row names the purses it can charge by the words the
    /// provider's card uses, grouped by provider, and leaves out a provider
    /// none of whose purses it can charge.
    #[test]
    fn an_agent_says_what_it_charges_in_the_providers_words() {
        let providers = vec![
            ProviderView {
                id: "anthropic".to_owned(),
                name: "Anthropic".to_owned(),
                purses: vec![
                    purse("anthropic-key", "API key"),
                    purse("anthropic-subscription", "Subscription token"),
                ],
            },
            ProviderView {
                id: "other".to_owned(),
                name: "Other".to_owned(),
                purses: vec![purse("other-key", "API key")],
            },
        ];
        let agent = Agent {
            id: "claude".to_owned(),
            name: "Claude".to_owned(),
            description: "does the work".to_owned(),
            ready: false,
            purses: vec![
                "anthropic-key".to_owned(),
                "anthropic-subscription".to_owned(),
            ],
        };

        assert_eq!(
            charges_of(&agent, &providers),
            vec!["Anthropic: API key or Subscription token".to_owned()]
        );
    }
}

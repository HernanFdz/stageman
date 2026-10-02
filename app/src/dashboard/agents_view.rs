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

use super::error::{DashboardError, DashboardResult, Refusal};
use super::live::{Live, Reading, use_reading};
use crate::ui::{
    BESIDE, Badge, BadgeTone, Button, ButtonVariant, Card, EmptyState, Guide, Icon, Mark, Row,
    Rows, SecretBox, Skeleton, Tooltip,
};

pub use stageman_wire::{Agent, Agents, ProviderView, PurseKind, PurseView};

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

/// Holds a purse, or replaces the credential it holds, once its provider
/// has accepted it: the answer waits on the provider, for as long as a
/// person at a button is asked to.
///
/// Replacing rather than refusing when one is already held, because
/// rotating a credential is the ordinary reason to come back to this screen
/// and a separate verb for it would be ceremony; a replaced credential
/// reaches every agent charging the purse at its next turn.
///
/// # Errors
///
/// Fails if the purse is not one this build knows, if the credential is
/// empty, if it has the shape of the other box's, if its provider does not
/// accept it, or if the provider could not be asked — and keeps nothing in
/// any of those cases, so a purse already held stays as it was.
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

    rsx! {
        div { class: "flex flex-col gap-4",
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
                                        // Whose card it is, once, on its title: the
                                        // rows under it say which kind each is.
                                        mark: rsx! { Mark { agent: provider.id.clone(), size: 16 } },
                                        title: provider.name.clone(),
                                        note: "What a kit's work is charged to: a key, metered per token, or a \
                                               subscription, flat per month. Paste each into its own box.",
                                        badge: rsx! { Badge { "{held} of {total} held" } },
                                        Rows {
                                            for purse in provider.purses.iter() {
                                                Row { key: "{purse.id}",
                                                    PurseRow {
                                                        provider: provider.id.clone(),
                                                        purse: purse.clone(),
                                                        onchanged: move |fresh: Agents| listing.set(fresh),
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
                                Rows {
                                    for agent in shown.agents.iter() {
                                        Row { key: "{agent.id}",
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
///
/// **Saving waits on the provider**, which is asked about what was pasted
/// before any of it is kept — see
/// `docs/decisions/0086-a-kit-charges-a-purse-at-a-provider.md`. Save says
/// so while it waits, and what went wrong is said in the row, in the place
/// of the line that says what the purse is for: a problem takes the line's
/// place, per `docs/conventions.md` §3, and the row already names the purse.
/// Nothing is told to the page above but a listing that changed.
#[component]
fn PurseRow(provider: String, purse: PurseView, onchanged: EventHandler<Agents>) -> Element {
    let mut credential = use_signal(String::new);
    let mut replacing = use_signal(|| false);
    // Whether the provider is being asked about what was pasted, and what
    // the last thing tried on this row was refused with.
    let mut checking = use_signal(|| false);
    let mut problem = use_signal(|| None::<DashboardError>);
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
    // Pressed on the button and on Enter in the box alike. Through the hook
    // rather than made here, because a callback made in a component's body
    // is kept until the component goes, and this body runs on every key.
    let save = {
        let identifier = identifier.clone();
        use_callback(move |()| {
            let supplied = credential();
            if held_back(&supplied, checking()) {
                return;
            }
            let identifier = identifier.clone();
            checking.set(true);
            problem.set(None);
            spawn(async move {
                let outcome = hold(identifier, supplied).await;
                checking.set(false);
                match outcome {
                    Ok(fresh) => {
                        credential.set(String::new());
                        replacing.set(false);
                        onchanged.call(fresh);
                    }
                    Err(why) => problem.set(Some(why)),
                }
            });
        })
    };

    rsx! {
        div { class: "flex flex-col gap-2",
            div { class: "flex items-center gap-3",
                {kind_icon(purse.kind).draw(14)}
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
            // One line, and a problem takes its place: what the last thing
            // tried here was refused with, or what the purse is for. The
            // problem runs the row's width rather than a paragraph's: it
            // names what a credential begins with, a browser breaks a line
            // at a hyphen, and at a paragraph's width the break fell inside
            // the very prefix the sentence is there to teach.
            if let Some(why) = problem() {
                p { role: "alert", class: "text-xs text-failed", "{row_says(&why)}" }
            } else {
                p { class: "max-w-prose text-xs text-muted-foreground",
                    "{purse.note} Charged by {charged_by_agents}."
                }
            }
            div { class: "flex flex-wrap items-center gap-2",
                if pasting {
                    SecretBox {
                        class: "max-w-sm text-xs",
                        placeholder: purse.example.clone(),
                        aria_label: "The {purse.name}",
                        value: credential(),
                        oninput: move |event: FormEvent| credential.set(event.value()),
                        onkeydown: move |event: KeyboardEvent| {
                            if event.key() == Key::Enter {
                                event.prevent_default();
                                save.call(());
                            }
                        },
                    }
                    // Not disabled while the provider is asked, though it
                    // says so: a control disabled under the pointer drops
                    // its focus. The guard in the save is what stops a
                    // second press.
                    Button {
                        disabled: credential().trim().is_empty(),
                        onclick: move |_| save.call(()),
                        if checking() { "Checking…" } else { "Save" }
                    }
                    if purse.held {
                        // Greyed while the provider is asked: a save that
                        // has gone cannot be called back, and pressing this
                        // then would say the old one was kept while the new
                        // one was on its way to replacing it.
                        Button {
                            variant: ButtonVariant::Secondary,
                            disabled: checking(),
                            onclick: move |_| {
                                credential.set(String::new());
                                replacing.set(false);
                                problem.set(None);
                            },
                            "Keep the old one"
                        }
                    }
                } else {
                    Button {
                        variant: ButtonVariant::Secondary,
                        onclick: move |_| {
                            problem.set(None);
                            replacing.set(true);
                        },
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
                                async move {
                                    match forget(identifier).await {
                                        Ok(fresh) => {
                                            problem.set(None);
                                            onchanged.call(fresh);
                                        }
                                        Err(why) => problem.set(Some(why)),
                                    }
                                }
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
        div { class: "flex flex-col gap-1",
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

/// Whether a press of Save is held back: nothing was pasted, or the provider
/// is already being asked about what was.
///
/// Pure, with its table below, because the press it guards happens only in
/// a page that is awake, where no test of a page reaches.
fn held_back(pasted: &str, checking: bool) -> bool {
    pasted.trim().is_empty() || checking
}

/// What a purse's row says, in the place of the line that says what the
/// purse is for, when something tried on it did not work.
///
/// The rule alone where the refusal is about what was pasted, since the row
/// already names the purse, and as a sentence, since a refusal's own words
/// are a clause; anything else in the words it came with.
fn row_says(failure: &DashboardError) -> String {
    match failure {
        DashboardError::Refused(
            Refusal::PurseMisshapen { rule: why, .. } | Refusal::PurseRefused { why, .. },
        ) => format!("Not kept: {why}."),
        DashboardError::Refused(Refusal::PurseUnchecked { why, .. }) => {
            format!("Not kept, because it could not be checked: {why}.")
        }
        other => other.to_string(),
    }
}

/// The icon a purse's row is drawn with: what kind it is, where the card's
/// title already says whose.
const fn kind_icon(kind: PurseKind) -> Icon {
    match kind {
        PurseKind::Key => Icon::Key,
        PurseKind::Subscription => Icon::Subscription,
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
    use super::{
        Agent, DashboardError, Icon, ProviderView, PurseKind, PurseView, Refusal, charges_of,
        held_back, kind_icon, row_says,
    };

    /// Each kind of purse is drawn with its own icon, and neither with the
    /// other's.
    #[test]
    fn each_kind_of_purse_is_drawn_with_its_own_icon() {
        assert_eq!(kind_icon(PurseKind::Key), Icon::Key);
        assert_eq!(kind_icon(PurseKind::Subscription), Icon::Subscription);
    }

    /// A press of Save goes through only with something pasted and nothing
    /// already being asked about: every case, and space alone counting as
    /// nothing pasted.
    #[test]
    fn a_save_is_held_back_with_nothing_pasted_or_while_one_is_being_checked() {
        assert!(!held_back("sk-ant-api03-a-key", false));
        assert!(
            held_back("sk-ant-api03-a-key", true),
            "one is being checked"
        );
        assert!(held_back("", false), "nothing was pasted");
        assert!(held_back("  \n", false), "space is nothing pasted");
        assert!(held_back("", true));
    }

    /// What a purse's row says when a paste was not kept, asserted whole per
    /// `docs/conventions.md` §4: the rule alone, since the row already names
    /// the purse, and as a sentence. Anything else is said in the words it
    /// came with.
    #[test]
    fn a_purses_row_says_the_rule_alone_and_as_a_sentence() {
        let said = |refusal: Refusal| row_says(&DashboardError::Refused(refusal));

        assert_eq!(
            said(Refusal::PurseRefused {
                purse: "Anthropic key".to_owned(),
                why: "Anthropic does not accept it: API key is invalid".to_owned(),
            }),
            "Not kept: Anthropic does not accept it: API key is invalid."
        );
        assert_eq!(
            said(Refusal::PurseMisshapen {
                purse: "Anthropic key".to_owned(),
                rule: "an Anthropic API key begins with sk-ant-api; a subscription's token goes \
                       in the other box"
                    .to_owned(),
            }),
            "Not kept: an Anthropic API key begins with sk-ant-api; a subscription's token goes \
             in the other box."
        );
        assert_eq!(
            said(Refusal::PurseUnchecked {
                purse: "Anthropic subscription".to_owned(),
                why: "Anthropic could not be reached: dns error".to_owned(),
            }),
            "Not kept, because it could not be checked: Anthropic could not be reached: dns \
             error."
        );
        assert_eq!(
            said(Refusal::PurseInUse {
                purse: "Anthropic key".to_owned(),
                by: vec!["the foreman of aviary".to_owned()],
            }),
            "Anthropic key is still charged by the foreman of aviary",
            "a refusal that is not about a paste, in its own words"
        );
        assert_eq!(
            row_says(&DashboardError::Failed),
            "that did not work — the server log says why"
        );
    }

    fn purse(id: &str, name: &str) -> PurseView {
        PurseView {
            id: id.to_owned(),
            name: name.to_owned(),
            kind: PurseKind::Key,
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

    // On the daemon's half only, for the reason the icon tests give.
    #[cfg(feature = "server")]
    mod drawn {
        use super::super::{PurseRow, PurseView};
        use super::purse;
        use dioxus::prelude::*;

        /// A purse as a row is drawn from one: an Anthropic key Claude can
        /// charge, held or not, charged by whatever is given.
        fn a_key(held: bool, charged_by: &[&str]) -> PurseView {
            PurseView {
                note: "Metered per token.".to_owned(),
                example: "sk-ant-api03-…".to_owned(),
                held,
                charged_by: charged_by.iter().map(|by| (*by).to_owned()).collect(),
                agents: vec!["Claude".to_owned()],
                ..purse("anthropic-key", "API key")
            }
        }

        /// A page holding one row, so that its handler is made where a
        /// handler can be: inside something being rendered.
        #[component]
        fn Page(purse: PurseView) -> Element {
            rsx! {
                PurseRow { provider: "anthropic", purse, onchanged: |_| {} }
            }
        }

        fn drawn(purse: PurseView) -> String {
            let mut dom = VirtualDom::new_with_props(Page, PageProps { purse });
            dom.rebuild_in_place();
            dioxus::ssr::render(&dom)
        }

        /// The attribute a control that refuses to be pressed carries, as
        /// the server writes it: every button's classes say `disabled:` of
        /// their own, so the word alone would match a control that is not.
        const DISABLED: &str = "disabled=true";

        /// A purse that is not held offers its box, showing what one of its
        /// kind begins with, and a Save with nothing yet to save; its one
        /// line says what it is for, and nothing on it is an alarm.
        #[test]
        fn a_purse_not_held_offers_its_box_and_says_what_it_is_for() {
            let row = drawn(a_key(false, &[]));

            assert!(row.contains("not held"), "{row}");
            assert!(row.contains(r#"placeholder="sk-ant-api03-…""#), "{row}");
            assert!(row.contains("Save"), "{row}");
            assert!(row.contains(DISABLED), "nothing to save yet: {row}");
            assert!(
                row.contains("Metered per token. Charged by Claude."),
                "{row}"
            );
            assert!(!row.contains(r#"role="alert""#), "{row}");
            assert!(!row.contains("Checking…"), "{row}");
            assert!(!row.contains("Replace"), "{row}");
            assert!(!row.contains("Keep the old one"), "{row}");
        }

        /// A purse that is held offers replacing and forgetting in the
        /// box's place, and forgetting is greyed, with what charges it,
        /// while something does.
        #[test]
        fn a_purse_held_offers_replacing_and_forgetting_in_the_boxs_place() {
            let free = drawn(a_key(true, &[]));
            assert!(free.contains("Replace"), "{free}");
            assert!(free.contains("Forget this purse"), "{free}");
            assert!(!free.contains("placeholder"), "no box: {free}");
            assert!(!free.contains("in use"), "{free}");
            assert!(!free.contains(DISABLED), "{free}");

            let charged = drawn(a_key(true, &["the foreman of aviary"]));
            assert!(charged.contains("in use"), "{charged}");
            assert!(
                charged.contains("Charged by the foreman of aviary, so it cannot be forgotten"),
                "{charged}"
            );
            assert!(charged.contains(DISABLED), "{charged}");
        }
    }
}

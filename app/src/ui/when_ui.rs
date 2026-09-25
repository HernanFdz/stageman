//! A moment, shown as how long ago it was — or, for one ahead, how long
//! until it is.
//!
//! The exact time is what the server renders and the browser hydrates, and
//! the relative reading is drawn afterwards — the rule in
//! `docs/conventions.md` §3: a relative time computed on two clocks is two
//! strings, which the framework reports as a mismatch. The exact time stays a
//! hover away, spelled the way the browser's own locale spells one once the
//! page is awake, and as the wire spells it before.

use dioxus::prelude::*;

use super::Tooltip;

/// Properties for [`When`].
#[derive(Props, PartialEq, Eq, Clone)]
pub struct WhenProps {
    /// The moment, as the wire spells it.
    pub at: String,
    /// Whether the moment may lie ahead: read as *in three days* then, and
    /// as *three days ago* once it has passed. A moment that is only ever
    /// behind reads as behind whatever a clock says.
    #[props(default)]
    pub ahead: bool,
    /// The classes the moment is drawn in, where the caller's are not the
    /// faint small ones a row's time takes.
    #[props(default = "font-mono text-xs text-faint-foreground".to_owned())]
    pub class: String,
}

/// How long ago `at` was, or how long until it is, once the page is awake;
/// the moment itself before.
#[component]
pub fn When(props: WhenProps) -> Element {
    let mut asked_for = use_signal(|| None::<(i64, String)>);
    let at = props.at.clone();

    use_effect(move || {
        let at = at.clone();
        spawn(async move {
            // On the server there is no evaluator and this is an error, which
            // leaves the exact moment showing.
            let mut asked = document::eval(&asking(&at));
            if let Ok(answered) = asked.recv::<(i64, String)>().await {
                asked_for.set(Some(answered));
            }
        });
    });

    let ahead = props.ahead;
    let (shown, exact) = asked_for().map_or_else(
        || (props.at.clone(), props.at.clone()),
        |(seconds, spelled)| (since(seconds, ahead), spelled),
    );

    rsx! {
        Tooltip { text: "{exact}",
            time {
                datetime: "{props.at}",
                class: "{props.class}",
                "{shown}"
            }
        }
    }
}

/// What the browser is asked: how many whole seconds ago `at` was — fewer
/// than none for a moment ahead — and the moment as the browser's own
/// locale spells one.
///
/// The browser's clock and the browser's parser. The fraction of a second is
/// dropped first, because not every parser takes six digits of it. The
/// answer is sent through the channel and never returned, per
/// `docs/conventions.md` §3.
fn asking(at: &str) -> String {
    format!(
        "var at = new Date(Date.parse({at:?}.replace(/\\.\\d+(?=Z$)/, ''))); \
         dioxus.send([Math.floor((Date.now() - at.getTime()) / 1000), \
         at.toLocaleString(undefined, {{ dateStyle: 'medium', timeStyle: 'short' }})]);"
    )
}

/// The words for a moment `seconds` behind: ahead where that is allowed
/// and the count says so, and otherwise behind — a clock that is behind
/// reads a moment only ever behind as *just now* rather than as the
/// future.
fn since(seconds: i64, ahead: bool) -> String {
    match (ahead, u64::try_from(seconds)) {
        (_, Ok(behind)) => ago(behind),
        (true, Err(_)) => until(seconds.unsigned_abs()),
        (false, Err(_)) => ago(0),
    }
}

const MINUTE: u64 = 60;
const HOUR: u64 = 60 * MINUTE;
const DAY: u64 = 24 * HOUR;
const MONTH: u64 = 30 * DAY;
const YEAR: u64 = 365 * DAY;
/// Up to here it is *just now*.
const MOMENTS: u64 = 45;
/// Up to here it is minutes.
const MINUTES: u64 = 45 * MINUTE;
/// Up to here it is hours.
const HOURS: u64 = 22 * HOUR;
/// Up to here it is days.
const DAYS: u64 = 26 * DAY;
/// Up to here it is months.
const MONTHS: u64 = 320 * DAY;

/// How long ago, in the coarsest words that are still true.
///
/// Rounded to the nearest unit, so that ninety seconds reads as two minutes
/// rather than one. A sum that will not fit is a moment older than this
/// program, which reads as *long ago* rather than as a wrong number.
#[must_use]
pub fn ago(seconds: u64) -> String {
    let counted = |units: Option<u64>, one: &str, many: &str| match units {
        None => "long ago".to_owned(),
        Some(1) => one.to_owned(),
        Some(n) => format!("{n} {many} ago"),
    };
    match seconds {
        s if s < MOMENTS => "just now".to_owned(),
        s if s < MINUTES => counted(rounded(seconds, MINUTE), "a minute ago", "minutes"),
        s if s < HOURS => counted(rounded(seconds, HOUR), "an hour ago", "hours"),
        s if s < DAYS => counted(rounded(seconds, DAY), "a day ago", "days"),
        s if s < MONTHS => counted(rounded(seconds, MONTH), "a month ago", "months"),
        _ => counted(rounded(seconds, YEAR), "a year ago", "years"),
    }
}

/// How long until, in the same words the other way: *in three days*.
///
/// A sum that will not fit is a moment further off than this program will
/// see, which reads as *far ahead* rather than as a wrong number.
#[must_use]
pub fn until(seconds: u64) -> String {
    let counted = |units: Option<u64>, one: &str, many: &str| match units {
        None => "far ahead".to_owned(),
        Some(1) => one.to_owned(),
        Some(n) => format!("in {n} {many}"),
    };
    match seconds {
        s if s < MOMENTS => "now".to_owned(),
        s if s < MINUTES => counted(rounded(seconds, MINUTE), "in a minute", "minutes"),
        s if s < HOURS => counted(rounded(seconds, HOUR), "in an hour", "hours"),
        s if s < DAYS => counted(rounded(seconds, DAY), "in a day", "days"),
        s if s < MONTHS => counted(rounded(seconds, MONTH), "in a month", "months"),
        _ => counted(rounded(seconds, YEAR), "in a year", "years"),
    }
}

/// How many of a unit `seconds` is, to the nearest.
fn rounded(seconds: u64, unit: u64) -> Option<u64> {
    seconds
        .checked_add(unit / 2)
        .and_then(|sum| sum.checked_div(unit))
}

#[cfg(test)]
mod tests {
    use super::{DAY, HOUR, MINUTE, ago, asking, since, until};

    /// The script sends its answer and never returns it, and quotes the
    /// moment as a string the browser can parse.
    #[test]
    fn the_browser_is_asked_by_a_script_that_sends_rather_than_returns() {
        let script = asking("2026-09-15T17:37:05.271648Z");
        assert!(script.contains("dioxus.send(["), "{script}");
        assert!(!script.contains("return"), "{script}");
        assert!(
            script.contains(r#""2026-09-15T17:37:05.271648Z""#),
            "{script}"
        );
    }

    /// The words at each boundary, which is where rounding either reads
    /// naturally or does not.
    #[test]
    fn a_moment_reads_in_the_coarsest_true_words() {
        assert_eq!(ago(0), "just now");
        assert_eq!(ago(44), "just now");
        assert_eq!(ago(45), "a minute ago");
        assert_eq!(ago(89), "a minute ago");
        assert_eq!(ago(90), "2 minutes ago");
        assert_eq!(ago(44 * MINUTE), "44 minutes ago");
        assert_eq!(ago(45 * MINUTE), "an hour ago");
        assert_eq!(ago(89 * MINUTE), "an hour ago");
        assert_eq!(ago(90 * MINUTE), "2 hours ago");
        assert_eq!(ago(21 * HOUR), "21 hours ago");
        assert_eq!(ago(22 * HOUR), "a day ago");
        assert_eq!(ago(36 * HOUR), "2 days ago");
        assert_eq!(ago(25 * DAY), "25 days ago");
        assert_eq!(ago(26 * DAY), "a month ago");
        assert_eq!(ago(200 * DAY), "7 months ago");
        assert_eq!(ago(320 * DAY), "a year ago");
        assert_eq!(ago(800 * DAY), "2 years ago");
    }

    /// The one input the arithmetic cannot take is answered with words rather
    /// than with a number that is wrong.
    #[test]
    fn a_moment_older_than_the_arithmetic_reads_as_long_ago() {
        assert_eq!(ago(u64::MAX), "long ago");
        assert_eq!(until(u64::MAX), "far ahead");
    }

    /// A moment ahead reads as *in*, the same words the other way; and a
    /// moment only ever behind reads as *just now* whatever a clock that
    /// is behind says.
    #[test]
    fn a_moment_ahead_reads_as_in_and_one_behind_as_ago() {
        assert_eq!(until(0), "now");
        assert_eq!(until(44), "now");
        assert_eq!(until(45), "in a minute");
        assert_eq!(until(90), "in 2 minutes");
        assert_eq!(until(44 * MINUTE), "in 44 minutes");
        assert_eq!(until(45 * MINUTE), "in an hour");
        assert_eq!(until(21 * HOUR), "in 21 hours");
        assert_eq!(until(22 * HOUR), "in a day");
        assert_eq!(until(3 * DAY), "in 3 days");
        assert_eq!(until(25 * DAY), "in 25 days");
        assert_eq!(until(26 * DAY), "in a month");
        assert_eq!(until(200 * DAY), "in 7 months");
        assert_eq!(until(319 * DAY), "in 11 months");
        assert_eq!(until(320 * DAY), "in a year");
        assert_eq!(until(800 * DAY), "in 2 years");
        let three_days = i64::try_from(3 * DAY).expect("fits");
        assert_eq!(since(three_days, true), "3 days ago");
        assert_eq!(since(-three_days, true), "in 3 days");
        assert_eq!(since(-three_days, false), "just now");
        assert_eq!(since(90, false), "2 minutes ago");
    }
}

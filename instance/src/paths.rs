//! Where an instance keeps what it keeps, and what it answers on, decided
//! from the environment it was given.
//!
//! The platform's rules for where configuration and data go, written out
//! rather than taken from a library, because the instance may read only the
//! map it was constructed with and a library reads the process's own. The
//! rules are the ones the library this replaced applied — see
//! `docs/decisions/0037-the-instance-key-is-generated-on-first-run.md` for
//! which directory is which and why.
//!
//! **macOS uses the same directories as Linux**, and that is the one thing
//! here that everybody guesses wrong, including the library: the strategy
//! this reproduces is the convention for a program run from a terminal, and
//! it is XDG everywhere except Windows. Apple's own directories were one
//! function call away in the library and are one `cfg` away here, and
//! reaching for either moves an instance that already exists — which, from
//! the outside, is indistinguishable from a first run, and mints a new key
//! rather than reporting anything.

use std::path::PathBuf;

use stageman_agent::Target;
use stageman_vocabulary::Environment;

use crate::tunnel::Domain;

/// The variable that overrides where the instance's file is.
///
/// An override rather than a requirement: where an instance lives is an
/// operational detail rather than a choice anybody should have to make, so
/// there is a per-platform default and this exists for the cases that
/// genuinely differ — a second instance on one machine, and a test that must
/// not touch the real one.
pub const STATE_VARIABLE: &str = "STAGEMAN_STATE";

/// The variable the file's encryption key arrives in, as base64.
pub const KEY_VARIABLE: &str = "STAGEMAN_KEY";

/// What names the domain this instance answers on.
pub const DOMAIN_VARIABLE: &str = "STAGEMAN_DOMAIN";

/// What names a different port for the tools a container reaches.
pub const TOOLS_VARIABLE: &str = "STAGEMAN_JOB_PORT";

/// The port the tools are served on when nothing says otherwise.
///
/// High and unusual, because the point is to collide with nothing. It is not
/// configuration in any meaningful sense — nobody needs to know it, and
/// nothing is served there that a person would visit — but a port can always
/// collide with something already running, so there is a way out.
const DEFAULT_TOOLS_PORT: u16 = 47_113;

/// The directory this instance's files go in, under the platform's own.
const INSTANCE_DIRECTORY: &str = "stageman";

/// What the instance's file is called.
const INSTANCE_FILE: &str = "instance.json";

/// What the generated key is called, in the platform's configuration
/// directory. A different directory from the instance, not merely a different
/// name: a key beside the file it protects protects nothing.
const KEY_FILE: &str = "key";

/// There is no home directory to put an instance under.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("no home directory, so there is nowhere to keep an instance — set {STATE_VARIABLE}")]
pub struct NoHome;

/// What a variable of this project's says, when it says anything.
///
/// Trimmed, and set-but-empty counts as unset: a variable a wrapper script
/// cleared means *do not use this* rather than a value of no characters, and
/// a key or a path arriving with a newline around it came from a shell rather
/// than from anybody's intent. Refusing to start over either would be a start
/// refused for something nobody wrote.
///
/// Only this project's own variables, and never the platform's: `HOME` and
/// the XDG pair are read exactly as the platform gives them, because a
/// directory whose name ends in a space is somebody's business and not ours.
fn said<'a>(environment: &'a Environment, named: &str) -> Option<&'a str> {
    environment
        .get(named)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
}

/// What a variable of this project's says, for whoever is not in this module.
#[must_use]
pub fn told(environment: &Environment, named: &str) -> Option<String> {
    said(environment, named).map(ToOwned::to_owned)
}

/// The user's home, as the platform names it.
fn home(environment: &Environment, target: Target) -> Result<PathBuf, NoHome> {
    let named = if target == Target::Windows {
        "USERPROFILE"
    } else {
        "HOME"
    };
    environment
        .get(named)
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .ok_or(NoHome)
}

/// An override that is honoured only when it is absolute, which is the
/// rule the specification gives.
fn absolute(environment: &Environment, named: &str) -> Option<PathBuf> {
    environment
        .get(named)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

/// Where the platform keeps a program's configuration.
///
/// Two cases and not three: see the note at the top of this module for why
/// macOS is not one of them.
fn configuration(environment: &Environment, target: Target) -> Result<PathBuf, NoHome> {
    let home = home(environment, target)?;
    if target == Target::Windows {
        Ok(
            absolute(environment, "APPDATA")
                .unwrap_or_else(|| home.join("AppData").join("Roaming")),
        )
    } else {
        Ok(absolute(environment, "XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config")))
    }
}

/// Where the platform keeps a program's data.
///
/// Windows defines this as the same place as the configuration directory and
/// this returns it, which 0037 records as a documented consequence rather
/// than something to work around. Everywhere else the two differ, which is
/// what keeps a generated key from sitting beside the file it protects.
fn data(environment: &Environment, target: Target) -> Result<PathBuf, NoHome> {
    let home = home(environment, target)?;
    if target == Target::Windows {
        Ok(
            absolute(environment, "APPDATA")
                .unwrap_or_else(|| home.join("AppData").join("Roaming")),
        )
    } else {
        Ok(absolute(environment, "XDG_DATA_HOME")
            .unwrap_or_else(|| home.join(".local").join("share")))
    }
}

/// Where the generated key is kept.
///
/// # Errors
///
/// Fails if there is no home directory to derive it from.
pub fn key_file(environment: &Environment, target: Target) -> Result<PathBuf, NoHome> {
    Ok(configuration(environment, target)?
        .join(INSTANCE_DIRECTORY)
        .join(KEY_FILE))
}

/// Where the instance's file is.
///
/// # Errors
///
/// Fails if nothing names it and there is no home directory to derive it
/// from.
pub fn instance_file(environment: &Environment, target: Target) -> Result<PathBuf, NoHome> {
    if let Some(named) = said(environment, STATE_VARIABLE) {
        return Ok(PathBuf::from(named));
    }
    Ok(data(environment, target)?
        .join(INSTANCE_DIRECTORY)
        .join(INSTANCE_FILE))
}

/// Which domain to answer on, given what the environment said.
///
/// Anything unreadable falls back rather than failing, for the reason the job
/// endpoint's port does the same: a mistyped value should not stop an instance
/// starting, because a tunnel is not what an operator came for. It is printed
/// at startup either way, so the fallback is visible rather than silent — and
/// here that matters more than it does for a port, because the fallback is a
/// working domain rather than an obviously wrong one.
#[must_use]
pub fn domain(environment: &Environment) -> Domain {
    said(environment, DOMAIN_VARIABLE)
        .and_then(Domain::parse)
        .unwrap_or_else(Domain::local)
}

/// The address a person reaches this instance on.
///
/// The framework's own rule, restated because the instance may read only the
/// map it was constructed with: two variables spelled exactly as the
/// framework's tooling sets them, and its defaults when neither is there.
/// Restating somebody else's rule is drift waiting to happen, and what keeps
/// it honest is that the integration tests run this binary and set both.
///
/// A port of zero is honoured here as it is everywhere else: whichever is
/// free, learned from the bind rather than assumed, which is what lets every
/// test that runs the binary have one of its own.
#[must_use]
pub fn dashboard_address(environment: &Environment) -> String {
    let host = said(environment, "IP").unwrap_or("127.0.0.1");
    let port = said(environment, "PORT")
        .and_then(|named| named.parse::<u16>().ok())
        .unwrap_or(8080);
    // A v6 address is bracketed in an address and bare in the variable.
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// Which port to serve the tools on, given what the environment said.
///
/// Anything unreadable falls back rather than failing, and that is
/// deliberate: a mistyped port should not stop an instance starting, because
/// the endpoint is not what an operator came for. It is named at startup
/// either way, so a fallback is visible rather than silent.
///
/// **Zero is honoured, and means whichever port is free.** No container can
/// be told about a port chosen after it was asked for — except that one is,
/// now: the address a container is given is composed from the port that was
/// actually taken. What wants this is a test that runs the binary, which
/// would otherwise fight over one fixed port with every other such test and
/// with whatever instance the operator is running. That is not hypothetical:
/// a leaked mutation-testing process held this port and a real daemon quietly
/// could not bind it.
#[must_use]
pub fn tools_port(environment: &Environment) -> u16 {
    said(environment, TOOLS_VARIABLE)
        .and_then(|named| named.parse::<u16>().ok())
        .unwrap_or(DEFAULT_TOOLS_PORT)
}

/// Where a container reaches the tools this instance serves.
///
/// One hostname for both runtimes, which is measured rather than assumed:
/// `--add-host=host.docker.internal:host-gateway` is honoured by Docker and
/// by Podman alike, so nothing here has to know which one is in use. The
/// port is the one that was actually taken rather than the one asked for,
/// which is the whole difference a port of zero makes.
#[must_use]
pub fn tools_endpoint(port: u16) -> String {
    format!("http://host.docker.internal:{port}/mcp")
}

/// Where a job's wrapper fetches its credential from: the same listener the
/// tools are served on, as a container reaches it — see
/// `docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`.
#[must_use]
pub fn credential_endpoint(port: u16) -> String {
    format!(
        "http://host.docker.internal:{port}{}",
        crate::tools::CREDENTIAL_PATH
    )
}

#[cfg(test)]
mod tests {
    use super::{Domain, NoHome, Target, domain, instance_file, key_file};
    use stageman_vocabulary::Environment;
    use std::path::PathBuf;

    fn environment(pairs: &[(&str, &str)]) -> Environment {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    /// The file is where it is told to be, and otherwise under the
    /// platform's data directory; the key is never beside it.
    #[test]
    fn the_file_and_the_key_go_where_the_platform_says_unless_told() {
        let told = environment(&[
            ("HOME", "/home/somebody"),
            ("STAGEMAN_STATE", "/elsewhere/i.json"),
        ]);
        assert_eq!(
            instance_file(&told, Target::Linux),
            Ok(PathBuf::from("/elsewhere/i.json")),
            "an override is honoured whole"
        );

        let home = environment(&[("HOME", "/home/somebody")]);
        let file = instance_file(&home, Target::Linux).expect("a home is enough");
        let key = key_file(&home, Target::Linux).expect("a home is enough");
        assert!(file.starts_with("/home/somebody"), "{}", file.display());
        assert!(
            file.ends_with("stageman/instance.json"),
            "{}",
            file.display()
        );
        assert!(key.ends_with("stageman/key"), "{}", key.display());
        assert_ne!(
            file.parent(),
            key.parent(),
            "a key beside the file it protects protects nothing"
        );

        let nowhere = environment(&[("STAGEMAN_STATE", "")]);
        assert_eq!(instance_file(&nowhere, Target::Linux), Err(NoHome));
        assert_eq!(key_file(&nowhere, Target::Linux), Err(NoHome));
    }

    /// Every platform's answer, from whichever machine this runs on.
    ///
    /// The whole point of the target being a value rather than a compiled
    /// condition: what a mac does is checkable from a Linux box. The branch
    /// nobody here could execute is the one that shipped wrong — a port that
    /// reached for Apple's own directories, which would have moved an
    /// instance that already existed and looked exactly like a first run.
    #[test]
    fn each_platform_keeps_them_where_that_platform_says() {
        let home = environment(&[("HOME", "/home/x"), ("USERPROFILE", r"C:\Users\x")]);

        // A mac keeps them where a Linux does. Apple's own directories are
        // one call away and are not what a program run from a terminal uses.
        assert_eq!(
            instance_file(&home, Target::MacOs),
            Ok(PathBuf::from("/home/x/.local/share/stageman/instance.json"))
        );
        assert_eq!(
            key_file(&home, Target::MacOs),
            Ok(PathBuf::from("/home/x/.config/stageman/key"))
        );
        for same in [Target::Linux, Target::Unknown] {
            assert_eq!(
                instance_file(&home, same),
                instance_file(&home, Target::MacOs),
                "{same:?} keeps its instance where a mac does"
            );
            assert_eq!(
                key_file(&home, same),
                key_file(&home, Target::MacOs),
                "{same:?} keeps its key where a mac does"
            );
        }

        // Windows reads a different variable for the home, and defines the
        // two directories as one place — which
        // `docs/decisions/0037-the-instance-key-is-generated-on-first-run.md`
        // records as a consequence rather than something to work around.
        let file = instance_file(&home, Target::Windows).expect("a home");
        let key = key_file(&home, Target::Windows).expect("a home");
        assert!(file.starts_with(r"C:\Users\x"), "{}", file.display());
        assert_eq!(file.parent(), key.parent(), "one directory, not two");
        assert_eq!(
            instance_file(&environment(&[("HOME", "/home/x")]), Target::Windows),
            Err(NoHome),
            "and it reads that variable rather than the one beside it"
        );
    }

    /// The platform's own overrides are honoured only when absolute.
    #[test]
    fn a_relative_xdg_override_is_ignored_as_the_specification_says() {
        let absolute = environment(&[("HOME", "/home/x"), ("XDG_DATA_HOME", "/data")]);
        assert_eq!(
            instance_file(&absolute, Target::Linux),
            Ok(PathBuf::from("/data/stageman/instance.json"))
        );
        let relative = environment(&[("HOME", "/home/x"), ("XDG_DATA_HOME", "data")]);
        assert_eq!(
            instance_file(&relative, Target::Linux),
            Ok(PathBuf::from("/home/x/.local/share/stageman/instance.json"))
        );
    }

    /// A variable cleared by a wrapper script is not a value, and one with a
    /// shell's newline around it is not a different path.
    #[test]
    fn a_cleared_variable_is_unset_and_a_padded_one_is_trimmed() {
        let cleared = environment(&[("HOME", "/home/x"), ("STAGEMAN_STATE", "   ")]);
        let derived = instance_file(&environment(&[("HOME", "/home/x")]), Target::Linux);
        assert_eq!(
            instance_file(&cleared, Target::Linux),
            derived,
            "an emptied variable falls back to the platform's own place"
        );
        assert_eq!(
            instance_file(
                &environment(&[("STAGEMAN_STATE", " /elsewhere/i.json\n")]),
                Target::Linux
            ),
            Ok(PathBuf::from("/elsewhere/i.json"))
        );
    }

    /// The dashboard is where the framework's own rule says, and a v6
    /// address is bracketed so that it is an address rather than a guess.
    #[test]
    fn the_dashboard_is_where_the_frameworks_own_variables_say() {
        use super::dashboard_address;

        assert_eq!(dashboard_address(&environment(&[])), "127.0.0.1:8080");
        assert_eq!(
            dashboard_address(&environment(&[("IP", "0.0.0.0"), ("PORT", "3000")])),
            "0.0.0.0:3000"
        );
        assert_eq!(
            dashboard_address(&environment(&[("PORT", "0")])),
            "127.0.0.1:0",
            "zero is whichever is free"
        );
        assert_eq!(
            dashboard_address(&environment(&[("PORT", "not a port")])),
            "127.0.0.1:8080"
        );
        assert_eq!(
            dashboard_address(&environment(&[("IP", "::1"), ("PORT", "8080")])),
            "[::1]:8080"
        );
    }

    /// A mistyped port falls back, and zero is honoured.
    #[test]
    fn the_tools_port_is_what_the_environment_says_or_the_one_nothing_uses() {
        use super::{DEFAULT_TOOLS_PORT, credential_endpoint, tools_endpoint, tools_port};

        assert_eq!(tools_port(&environment(&[])), DEFAULT_TOOLS_PORT);
        assert_eq!(
            tools_port(&environment(&[("STAGEMAN_JOB_PORT", "not a port")])),
            DEFAULT_TOOLS_PORT
        );
        assert_eq!(
            tools_port(&environment(&[("STAGEMAN_JOB_PORT", "47114")])),
            47_114
        );
        assert_eq!(
            tools_port(&environment(&[("STAGEMAN_JOB_PORT", "0")])),
            0,
            "zero is a port a test asks for on purpose"
        );
        assert_eq!(
            tools_endpoint(47_113),
            "http://host.docker.internal:47113/mcp"
        );
        assert_eq!(
            credential_endpoint(47_113),
            "http://host.docker.internal:47113/credential"
        );
    }

    /// A mistyped domain falls back rather than stopping the instance.
    #[test]
    fn an_unreadable_domain_falls_back_to_the_default() {
        assert_eq!(domain(&environment(&[])), Domain::local());
        assert_eq!(
            domain(&environment(&[("STAGEMAN_DOMAIN", "not a domain")])),
            Domain::local()
        );
        assert_eq!(
            domain(&environment(&[("STAGEMAN_DOMAIN", "example.com")])),
            Domain::parse("example.com").expect("a domain"),
        );
    }
}

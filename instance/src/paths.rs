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
fn home(environment: &Environment) -> Result<PathBuf, NoHome> {
    let named = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
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
fn configuration(environment: &Environment) -> Result<PathBuf, NoHome> {
    let home = home(environment)?;
    if cfg!(windows) {
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
fn data(environment: &Environment) -> Result<PathBuf, NoHome> {
    let home = home(environment)?;
    if cfg!(windows) {
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
pub fn key_file(environment: &Environment) -> Result<PathBuf, NoHome> {
    Ok(configuration(environment)?
        .join(INSTANCE_DIRECTORY)
        .join(KEY_FILE))
}

/// Where the instance's file is.
///
/// # Errors
///
/// Fails if nothing names it and there is no home directory to derive it
/// from.
pub fn instance_file(environment: &Environment) -> Result<PathBuf, NoHome> {
    if let Some(named) = said(environment, STATE_VARIABLE) {
        return Ok(PathBuf::from(named));
    }
    Ok(data(environment)?
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

#[cfg(test)]
mod tests {
    use super::{Domain, NoHome, domain, instance_file, key_file};
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
            instance_file(&told),
            Ok(PathBuf::from("/elsewhere/i.json")),
            "an override is honoured whole"
        );

        let home = environment(&[("HOME", "/home/somebody")]);
        let file = instance_file(&home).expect("a home is enough");
        let key = key_file(&home).expect("a home is enough");
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
        assert_eq!(instance_file(&nowhere), Err(NoHome));
        assert_eq!(key_file(&nowhere), Err(NoHome));
    }

    /// A variable cleared by a wrapper script is not a value, and one with a
    /// shell's newline around it is not a different path.
    #[test]
    fn a_cleared_variable_is_unset_and_a_padded_one_is_trimmed() {
        let cleared = environment(&[("HOME", "/home/x"), ("STAGEMAN_STATE", "   ")]);
        let derived = instance_file(&environment(&[("HOME", "/home/x")]));
        assert_eq!(
            instance_file(&cleared),
            derived,
            "an emptied variable falls back to the platform's own place"
        );
        assert_eq!(
            instance_file(&environment(&[("STAGEMAN_STATE", " /elsewhere/i.json\n")])),
            Ok(PathBuf::from("/elsewhere/i.json"))
        );
    }

    /// The platform's own overrides are honoured only when absolute.
    #[cfg(not(windows))]
    #[test]
    fn a_relative_xdg_override_is_ignored_as_the_specification_says() {
        let absolute = environment(&[("HOME", "/home/x"), ("XDG_DATA_HOME", "/data")]);
        assert_eq!(
            instance_file(&absolute),
            Ok(PathBuf::from("/data/stageman/instance.json"))
        );
        let relative = environment(&[("HOME", "/home/x"), ("XDG_DATA_HOME", "data")]);
        assert_eq!(
            instance_file(&relative),
            Ok(PathBuf::from("/home/x/.local/share/stageman/instance.json"))
        );
    }

    /// A mac keeps them where a Linux does, and this is the test that says
    /// so on purpose.
    ///
    /// The guess everybody makes is Apple's own directories, and taking them
    /// would move an instance that exists — a running stageman would find
    /// nothing where it looked, mint a key, and write an empty instance over
    /// nobody's objection. Asserted on every platform but Windows, so the
    /// machine this runs on is not what decides whether it is checked.
    #[cfg(not(windows))]
    #[test]
    fn a_mac_keeps_them_where_a_linux_does_rather_than_under_its_own_library() {
        let home = environment(&[("HOME", "/home/x")]);
        assert_eq!(
            instance_file(&home),
            Ok(PathBuf::from("/home/x/.local/share/stageman/instance.json"))
        );
        assert_eq!(
            key_file(&home),
            Ok(PathBuf::from("/home/x/.config/stageman/key"))
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

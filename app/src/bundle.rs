//! The browser's half, carried inside the binary.
//!
//! What ships is one file. The alternative was a `public/` directory beside
//! the executable, which is two artifacts that can be separated, paired with
//! the wrong one, or copied without `-r` — and the failure when they are is a
//! page that renders and never comes alive, which looks almost exactly like
//! one that works. Reasoning and the three constraints that shaped it are in
//! `docs/decisions/0038-the-browsers-half-lives-in-the-binary.md`.
//!
//! **Empty is an ordinary state, not a failure.** A plain `cargo build`
//! embeds nothing, because the directory the build script reads is populated
//! only by `just build`. A binary with nothing here behaves exactly as one
//! did before any of this existed: it looks for a bundle beside itself, and
//! serves a complete, inert page if there is none.

// The table the build script wrote: `EMBEDDED`, one entry per file, sorted.
include!(concat!(env!("OUT_DIR"), "/bundle.rs"));

/// What the framework insists on reading from a directory.
///
/// Everything else here is served from memory. This one is not served at all
/// — it is parsed once into fragments and interleaved with the rendered page,
/// so it is an input to the renderer rather than a response, which is why it
/// cannot simply be handed out by the route below.
pub const INDEX: &str = "index.html";

/// A table of carried files, so that what reads one can be handed another.
///
/// A type rather than free functions over [`EMBEDDED`], and the reason is
/// testability rather than taste: the gate compiles this binary carrying
/// nothing, so a function reading the compiled-in table directly returns
/// nothing and agrees with every mutation of itself. Mutation testing found
/// four such functions here at once. Taking the table is what makes an
/// assertion about lookups an assertion rather than a tautology.
#[derive(Debug, Clone, Copy)]
pub struct Bundle(&'static [(&'static str, &'static [u8])]);

/// What this binary actually carries.
pub const CARRIED: Bundle = Bundle(EMBEDDED);

impl Bundle {
    /// A bundle over a table this module did not generate.
    ///
    /// The reason [`Bundle`] is a type at all: it lets a test hand the same
    /// code a table it can reason about, which the compiled-in one can never
    /// be in a build that carries nothing.
    ///
    /// Compiled only for tests, because that is the whole truth about it — the
    /// one bundle a running binary has is [`CARRIED`], and a constructor
    /// present in a release build would be an invitation to make a second.
    /// This module's own tests build one directly; this exists for the
    /// serving module's, which cannot reach a private field.
    #[cfg(test)]
    #[must_use]
    pub const fn of(table: &'static [(&'static str, &'static [u8])]) -> Self {
        Self(table)
    }

    /// Every entry, for whoever is building routes out of them.
    #[must_use]
    pub const fn entries(self) -> &'static [(&'static str, &'static [u8])] {
        self.0
    }

    /// The index, if there is one.
    #[must_use]
    pub fn index(self) -> Option<&'static [u8]> {
        self.file(INDEX)
    }

    /// One file, by the path it is served under.
    ///
    /// A linear scan over a handful of entries, which is faster than any map
    /// worth building for it and needs no dependency to say so.
    #[must_use]
    pub fn file(self, path: &str) -> Option<&'static [u8]> {
        self.0
            .iter()
            .find(|(served, _)| *served == path)
            .map(|(_, bytes)| *bytes)
    }
}

/// What a browser should be told a file is.
///
/// Wrong answers here do not look like errors. A stylesheet served as
/// `text/plain` is ignored, and a wasm module served as anything but its own
/// type is refused by the streaming compiler — both of which present as a
/// page that arrived and does nothing, rather than as a failure anybody sees.
///
/// The list is the extensions a Dioxus bundle actually contains, and anything
/// else is deliberately handed back as bytes rather than guessed at: a wrong
/// guess is silent, and this is not a general-purpose file server.
#[must_use]
pub fn content_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, extension)| extension) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/vnd.microsoft.icon",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::{Bundle, CARRIED, EMBEDDED, INDEX, content_type};

    /// A table of the shape a real bundle has, which a test can reason about.
    ///
    /// The compiled-in one is empty in the gate — it is only populated by
    /// `just build` — so every assertion about lookups has to be made against
    /// a table the test supplies, or it asserts that nothing maps to nothing.
    const CARRYING: Bundle = Bundle(&[
        ("index.html", b"<html></html>"),
        ("assets/app-abc.js", b"console.log(1)"),
        ("assets/app-abc.wasm", b"\0asm"),
    ]);

    /// What is carried is what was compiled in, and nothing else.
    #[test]
    fn a_bundle_holds_the_table_it_was_given() {
        assert_eq!(CARRYING.entries().len(), 3);
        assert!(Bundle(&[]).entries().is_empty());
        assert_eq!(
            CARRIED.entries(),
            EMBEDDED,
            "the real one is the real table"
        );
    }

    /// The index is found by its own name and nothing else is mistaken for it.
    #[test]
    fn the_index_is_found_by_name() {
        assert_eq!(CARRYING.index(), Some(&b"<html></html>"[..]));
        assert_eq!(Bundle(&[]).index(), None, "an empty bundle has no index");
        assert_eq!(INDEX, "index.html");
    }

    /// A lookup returns the bytes filed under that exact path.
    ///
    /// The exactness is the half worth asserting: a lookup that matched
    /// anything *but* the path asked for would still find something, and would
    /// serve one file under another's name.
    #[test]
    fn a_lookup_is_exact() {
        assert_eq!(
            CARRYING.file("assets/app-abc.js"),
            Some(&b"console.log(1)"[..])
        );
        assert_eq!(CARRYING.file("assets/app-abc.wasm"), Some(&b"\0asm"[..]));
        assert_eq!(
            CARRYING.file("assets/app-abc.j"),
            None,
            "not a prefix match"
        );
        assert_eq!(CARRYING.file("app-abc.js"), None, "not a suffix match");
        assert_eq!(CARRYING.file(""), None);
    }

    /// Every type this names is named, and everything else is bytes.
    ///
    /// One assertion per arm, because a deleted arm falls through to the
    /// fallback and produces exactly the silent failure this function exists to
    /// prevent — a file the browser fetches and then refuses to use.
    #[test]
    fn every_type_a_browser_refuses_to_guess_is_named() {
        assert_eq!(content_type("a.wasm"), "application/wasm");
        assert_eq!(content_type("a.js"), "text/javascript; charset=utf-8");
        assert_eq!(content_type("a.css"), "text/css; charset=utf-8");
        assert_eq!(content_type("a.html"), "text/html; charset=utf-8");
        assert_eq!(content_type("a.json"), "application/json");
        assert_eq!(content_type("a.svg"), "image/svg+xml");
        assert_eq!(content_type("a.png"), "image/png");
        assert_eq!(content_type("a.ico"), "image/vnd.microsoft.icon");
        assert_eq!(content_type("a.woff2"), "font/woff2");
        assert_eq!(content_type("a.bin"), "application/octet-stream");
        assert_eq!(content_type("assets"), "application/octet-stream");
    }
}

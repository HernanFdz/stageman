//! A `.env` file, read into a form's rows.
//!
//! What a browser does reliably with pasted text is put it in a text area,
//! so a file arrives as text and is read here into variable rows: one
//! `NAME=value` per line, as a shell reads one, with the comment above a
//! line as its note — see
//! `docs/decisions/0075-a-variable-says-what-it-is-for.md`. Pure, and
//! tested on the host, though it runs in the browser. What it cannot read
//! it hands back as it was, so that nothing pasted is dropped unseen; and it
//! refuses nothing itself, because what a name or a value may be is the
//! instance's to say, by row, when the form is saved.

use stageman_wire::VariableDraft;

/// What reading a pasted file produced.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Read {
    /// The rows it could read, in the file's order.
    pub variables: Vec<VariableDraft>,
    /// The lines it could not, as they were.
    pub left: Vec<String>,
}

/// Reads a file's text into rows.
///
/// A line that is `NAME=value` is a row, `export` before the name dropped,
/// the value unquoted as a shell would. The comment lines just above it are
/// its note, joined with spaces; a blank line parts a comment from what
/// follows, so a section's heading is nobody's note. Anything else is left
/// as it was.
#[must_use]
pub fn read(text: &str) -> Read {
    let mut read = Read::default();
    let mut note: Vec<String> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            note.clear();
            continue;
        }
        if let Some(comment) = trimmed.strip_prefix('#') {
            note.push(comment.trim().to_owned());
            continue;
        }
        match assignment(trimmed) {
            Some((name, value)) => {
                read.variables.push(VariableDraft {
                    name,
                    value,
                    note: note.join(" "),
                });
            }
            None => read.left.push(line.to_owned()),
        }
        note.clear();
    }
    read
}

/// The name and the value a line assigns, if it assigns one.
fn assignment(line: &str) -> Option<(String, String)> {
    let line = line.strip_prefix("export ").map_or(line, str::trim_start);
    let (name, value) = line.split_once('=')?;
    let name = name.trim();
    if name.is_empty() || name.chars().any(char::is_whitespace) {
        return None;
    }
    Some((name.to_owned(), unquoted(value.trim())))
}

/// A value as a shell would read it: double quotes with their escapes,
/// single quotes as they stand, either followed by nothing or a comment,
/// and an unquoted value up to a comment.
fn unquoted(value: &str) -> String {
    if let Some(inner) = quoted(value, '"') {
        return unescaped(inner);
    }
    if let Some(inner) = quoted(value, '\'') {
        return inner.to_owned();
    }
    value
        .split_once(" #")
        .map_or(value, |(before, _)| before)
        .trim_end()
        .to_owned()
}

/// What is inside a value's quotes, when the value is quoted with `quote`
/// and nothing but a comment follows the closing one. A backslash escapes
/// the next character inside double quotes, as a shell reads them.
fn quoted(value: &str, quote: char) -> Option<&str> {
    let rest = value.strip_prefix(quote)?;
    let mut escaped = false;
    for (at, character) in rest.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote == '"' {
            escaped = true;
            continue;
        }
        if character == quote {
            let (inner, after) = rest.split_at(at);
            let after = after.strip_prefix(quote)?.trim_start();
            return (after.is_empty() || after.starts_with('#')).then_some(inner);
        }
    }
    None
}

/// The escapes a double-quoted value carries: a newline, a quote, and a
/// backslash; anything else stays as written.
fn unescaped(inner: &str) -> String {
    let mut out = String::with_capacity(inner.len());
    let mut characters = inner.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        match characters.next() {
            Some('n') => out.push('\n'),
            Some('"') => out.push('"'),
            Some('\\') | None => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Read, read};
    use stageman_wire::VariableDraft;

    fn row(name: &str, value: &str, note: &str) -> VariableDraft {
        VariableDraft {
            name: name.to_owned(),
            value: value.to_owned(),
            note: note.to_owned(),
        }
    }

    /// The file an operator has, read as a shell would read it, with the
    /// comment above a line as its note.
    #[test]
    fn a_file_is_read_a_line_at_a_time_with_the_comment_above_as_its_note() {
        let text = "\
# the payment provider, in test mode
STRIPE_API_KEY=sk_test_not_a_real_key
export DATABASE_URL=\"postgres://nowhere/db?sslmode=require\"
TOKEN='keep \\n as it is' # not a note
EMPTY=
";
        assert_eq!(
            read(text),
            Read {
                variables: vec![
                    row(
                        "STRIPE_API_KEY",
                        "sk_test_not_a_real_key",
                        "the payment provider, in test mode"
                    ),
                    row("DATABASE_URL", "postgres://nowhere/db?sslmode=require", ""),
                    row("TOKEN", "keep \\n as it is", ""),
                    row("EMPTY", "", ""),
                ],
                left: Vec::new(),
            }
        );
    }

    /// Double quotes carry escapes and single quotes carry nothing, and a
    /// comment may follow either; an unquoted value ends at a comment, and
    /// spaces around the sign are nobody's. A quote that never closes, or
    /// closes before something that is not a comment, is not a quote.
    #[test]
    fn a_value_is_read_as_a_shell_would() {
        let text = concat!(
            "A=\"one\\ntwo \\\"quoted\\\" back\\\\slash\"\n",
            "B = spaced # trailing\n",
            "C=with # inside\n",
            "D='kept # inside' # a comment after the quote\n",
            "E=\"unclosed\n",
            "F='closed' then more\n",
        );
        let read = read(text);
        assert_eq!(
            read.variables,
            vec![
                row("A", "one\ntwo \"quoted\" back\\slash", ""),
                row("B", "spaced", ""),
                row("C", "with", ""),
                row("D", "kept # inside", ""),
                row("E", "\"unclosed", ""),
                row("F", "'closed' then more", ""),
            ]
        );
        assert!(read.left.is_empty());
    }

    /// Comment lines just above a line are its note, joined; a blank line
    /// parts them from what follows, so a heading is nobody's note.
    #[test]
    fn a_note_is_the_comments_just_above_and_a_blank_line_parts_them() {
        let read = read(
            "# Database\n\n# read-only,\n# the staging one\nDATABASE_URL=x\n# dangling\n\nOTHER=y\n",
        );
        assert_eq!(
            read.variables,
            vec![
                row("DATABASE_URL", "x", "read-only, the staging one"),
                row("OTHER", "y", ""),
            ]
        );
    }

    /// What cannot be read is handed back as it was, in order, so that
    /// nothing pasted is dropped unseen.
    #[test]
    fn what_cannot_be_read_is_left_as_it_was() {
        let found = read("A=1\nsource .env.local\n=nameless\nNOT A NAME=2\n  B=2  \n");
        assert_eq!(found.variables, vec![row("A", "1", ""), row("B", "2", "")]);
        assert_eq!(
            found.left,
            vec![
                "source .env.local".to_owned(),
                "=nameless".to_owned(),
                "NOT A NAME=2".to_owned()
            ]
        );
        assert_eq!(read(""), Read::default());
        assert_eq!(read("\n  \n# only a comment\n"), Read::default());
    }
}

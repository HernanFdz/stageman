#![doc = include_str!("../README.md")]

/// Returns the answer.
///
/// ```
/// assert_eq!(stageman::answer(), 42);
/// ```
#[must_use]
pub const fn answer() -> u32 {
    42
}

#[cfg(test)]
mod tests {
    use super::answer;

    #[test]
    fn the_answer_is_the_answer() {
        assert_eq!(answer(), 42);
    }
}

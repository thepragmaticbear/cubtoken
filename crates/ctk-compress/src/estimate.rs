//! Cheap token estimation: ~3.5 chars per token for code-like text.

pub fn est_tokens(s: &str) -> usize {
    (s.chars().count() as f32 / 3.5).ceil() as usize
}

#[cfg(test)]
mod tests {
    use super::est_tokens;

    #[test]
    fn estimates_by_chars() {
        assert_eq!(est_tokens(""), 0);
        assert_eq!(est_tokens(&"abcd".repeat(35)), 40); // 140 chars / 3.5
    }
}

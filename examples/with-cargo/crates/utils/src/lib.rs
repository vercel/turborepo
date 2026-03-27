pub fn pluralize(word: &str, count: u8) -> String {
    if count == 1 {
        word.to_string()
    } else {
        format!("{word}s")
    }
}

pub fn truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else {
        &s[..max_len]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pluralize_singular() {
        assert_eq!(pluralize("item", 1), "item");
    }

    #[test]
    fn pluralize_plural() {
        assert_eq!(pluralize("item", 5), "items");
    }

    #[test]
    fn truncate_short() {
        assert_eq!(truncate("hi", 10), "hi");
    }

    #[test]
    fn truncate_long() {
        assert_eq!(truncate("hello world", 5), "hello");
    }
}

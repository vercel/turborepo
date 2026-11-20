pub fn greeting() -> &'static str {
    "hello from lib-a"
}

#[cfg(test)]
mod tests {
    #[test]
    fn returns_greeting() {
        assert_eq!(
            lib_a_test_util::expected_greeting(),
            "hello from lib-a"
        );
    }
}

/// An iterator that returns increasingly less specific keys
pub struct PossibleKeyIter<'a> {
    key: Option<&'a str>,
}

impl<'a> PossibleKeyIter<'a> {
    pub fn new(key: &'a str) -> Self {
        Self { key: Some(key) }
    }
}

impl<'a> Iterator for PossibleKeyIter<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        let curr = self.key;
        self.key = curr.and_then(less_specific_key);
        curr
    }
}

fn less_specific_key(key: &str) -> Option<&str> {
    let slash_idx = key.find('/')?;
    let after_slash = &key[slash_idx + 1..];
    // We have a scope so ignore first '/' we find
    if key.starts_with('@') {
        let next_slash_idx = after_slash.find('/')?;
        Some(&after_slash[next_slash_idx + 1..])
    } else {
        Some(after_slash)
    }
}

#[cfg(test)]
mod test {
    use test_case::test_case;

    use super::*;

    #[test_case("top", None; "top")]
    #[test_case("@scope/top", None ; "scope")]
    #[test_case("parent/dep", Some("dep") ; "parent")]
    #[test_case("@scope/parent/dep", Some("dep") ; "scope parent")]
    #[test_case("gp/p/dep", Some("p/dep") ; "grandparent")]
    fn test_lsk(input: &str, expected: Option<&str>) {
        assert_eq!(less_specific_key(input), expected);
    }
}

use regex::Regex;

/// A simple glob-like pattern that supports a subset of
/// glob syntax for the purposes of string matching.
/// If you are matching paths, use `turborepo_wax::glob::Glob` instead.
pub enum SimpleGlob {
    Regex(Regex),
    String(String),
    Any,
}

pub trait Match {
    fn is_match(&self, s: &str) -> bool;
}

impl SimpleGlob {
    pub fn new(pattern: &str) -> Result<Self, regex::Error> {
        if pattern == "*" {
            Ok(SimpleGlob::Any)
        } else if pattern.contains('*') {
            let regex = Regex::new(&format!("^{}$", pattern.replace('*', ".*")))?;
            Ok(SimpleGlob::Regex(regex))
        } else {
            Ok(SimpleGlob::String(pattern.to_string()))
        }
    }
}

impl Match for SimpleGlob {
    fn is_match(&self, s: &str) -> bool {
        match self {
            SimpleGlob::Regex(regex) => regex.is_match(s),
            SimpleGlob::String(string) => string == s,
            SimpleGlob::Any => true,
        }
    }
}

pub struct AnyGlob<T: Match>(Vec<T>);

impl<T: Match> Match for AnyGlob<T> {
    fn is_match(&self, s: &str) -> bool {
        self.0.iter().any(|glob| glob.is_match(s))
    }
}

pub struct NotGlob<T: Match>(T);

impl<T: Match> Match for NotGlob<T> {
    fn is_match(&self, s: &str) -> bool {
        !self.0.is_match(s)
    }
}

pub struct IncludeExcludeGlob<I: Match, E: Match> {
    include: I,
    exclude: E,
}

impl IncludeExcludeGlob<AnyGlob<SimpleGlob>, AnyGlob<SimpleGlob>> {
    pub fn new_from_globs<'a>(
        include: impl Iterator<Item = &'a dyn AsRef<&'a str>>,
        exclude: impl Iterator<Item = &'a dyn AsRef<&'a str>>,
        _include_default: bool,
        _exclude_default: bool,
    ) -> Self {
        let include = AnyGlob(
            include
                .map(|glob| SimpleGlob::new(glob.as_ref()).unwrap())
                .collect(),
        );

        let exclude = AnyGlob(
            exclude
                .map(|glob| SimpleGlob::new(glob.as_ref()).unwrap())
                .collect(),
        );

        Self { include, exclude }
    }
}

impl<T: Match, E: Match> Match for IncludeExcludeGlob<T, E> {
    fn is_match(&self, s: &str) -> bool {
        self.include.is_match(s) && !self.exclude.is_match(s)
    }
}

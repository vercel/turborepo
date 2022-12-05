/// A predicate that can match any number of exports.
///
/// Note: There is no `All` variant because two different export predicates
/// cannot match the same export. See the documentation of
/// [`ExactExportPredicate`] for more details.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum ExportPredicate<'a> {
    /// Preserve exports that match any predicate in the set.
    Any(Vec<ExportPredicate<'a>>),
    /// Preserve exports that don't match the given predicate.
    Not(Box<ExportPredicate<'a>>),
    /// Only preserve the export that matches the given exact export predicate.
    Exact(ExactExportPredicate<'a>),
}

impl<'a> ExportPredicate<'a> {
    pub fn matches<'b>(&self, other: &ExactExportPredicate<'b>) -> bool {
        match self {
            ExportPredicate::Any(predicates) => {
                predicates.iter().any(|predicate| predicate.matches(other))
            }
            ExportPredicate::Not(predicate) => !predicate.matches(other),
            ExportPredicate::Exact(exact) => exact == other,
        }
    }
}

/// A predicate that can only match a single export.
///
/// Exact export predicates are injective. This means that if an export matches
/// predicate A, it will only match predicate B if
/// A == B.
#[derive(Clone, Debug, Hash, Eq, PartialEq)]
pub enum ExactExportPredicate<'a> {
    /// Matches the default export.
    Default,
    /// Matches a named export.
    Named(&'a str),
}

impl<'a> From<ExactExportPredicate<'a>> for ExportPredicate<'a> {
    fn from(exact: ExactExportPredicate<'a>) -> Self {
        ExportPredicate::Exact(exact)
    }
}

/// Will only preserve SSR and SSG exports.
/// Necessary for SSR/SSG HMR.
pub fn ssg_ssr_export_predicate() -> ExportPredicate<'static> {
    ExportPredicate::Any(vec![
        // SSR
        ExactExportPredicate::Named("getServerSideProps").into(),
        // SSG
        ExactExportPredicate::Named("getStaticProps").into(),
        ExactExportPredicate::Named("getStaticPaths").into(),
    ])
}

/// Will only preserve non-SSR and non-SSG exports.
/// Necessary for client-side rendering.
pub fn client_side_export_predicate() -> ExportPredicate<'static> {
    ExportPredicate::Not(Box::new(ssg_ssr_export_predicate()))
}

/// Will only preserve the default export and an export named "foo".
/// This is an example of what a production tree shaking pass could look like.
pub fn example_predicate() -> ExportPredicate<'static> {
    ExportPredicate::Any(vec![
        ExactExportPredicate::Default.into(),
        ExactExportPredicate::Named("foo").into(),
    ])
}

#[cfg(test)]
mod tests {
    use super::{client_side_export_predicate, ssg_ssr_export_predicate, ExactExportPredicate};
    use crate::example_predicate;

    #[test]
    fn ssg_ssr() {
        let p = ssg_ssr_export_predicate();
        assert!(p.matches(&ExactExportPredicate::Named("getServerSideProps")));
        assert!(p.matches(&ExactExportPredicate::Named("getStaticProps")));
        assert!(p.matches(&ExactExportPredicate::Named("getStaticPaths")));
        assert!(!p.matches(&ExactExportPredicate::Default));
        assert!(!p.matches(&ExactExportPredicate::Named("Home")));
    }

    #[test]
    fn client() {
        let p = client_side_export_predicate();
        assert!(p.matches(&ExactExportPredicate::Named("Home")));
        assert!(p.matches(&ExactExportPredicate::Default));
        assert!(!p.matches(&ExactExportPredicate::Named("getServerSideProps")));
        assert!(!p.matches(&ExactExportPredicate::Named("getStaticProps")));
        assert!(!p.matches(&ExactExportPredicate::Named("getStaticPaths")));
    }

    #[test]
    fn example() {
        let p = example_predicate();
        assert!(p.matches(&ExactExportPredicate::Default));
        assert!(p.matches(&ExactExportPredicate::Named("foo")));
        assert!(!p.matches(&ExactExportPredicate::Named("bar")));
    }
}

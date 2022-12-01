//! # Three-shaking
//!
//! ## Goals
//!
//! We want Turbopack's tree shaking to be more granular than that of Webpack.
//! While Webpack will eliminate unused exports across the whole compilation
//! from a module, this module might end up duplicated across chunks, each chunk
//! only needing part of that module.
//!
//! Instead, Tobias proposed Turbopack should split all module-level
//! declarations into their own modules. This way, each chunk can include only
//! the declarations it needs, and these declaration modules can be shared
//! across chunks.
//!
//! This tree shaking implementation is primarily a concern for production
//! builds. We don't need a fully-fledged implementation for development builds.
//!
//! However, we still need some form of tree shaking for eliminating
//! SSG and SSR specific functions from pages (`getServerSideProps`, etc.). This
//! is already implemented as the SSG transform (next_ssg.rs).
//!
//! Similarly, for HMRing SSG and SSR, we need the complementary of this
//! operation: we want to eliminate all but SSG and SSR functions, so we can
//! ensure that we only re-render and update the server-side representation
//! when SSG and SSR functions change.
//!
//! This requires a form of tree shaking more advanced than the one we currently
//! have in the SSG transform.
//!
//! ## Implementation
//!
//! ### Compiler passes
//!
//! The tree-shaking transform works in multiple passes:
//!
//! #### 1: The analyzer pass.
//!
//! This pass would build a directed, possibly-cyclic graph of dependencies
//! between identifiers in a module. The graph is built starting from exports
//! and leading back up to module declarations and imports.
//!
//! The analyzer pass can be restricted to only consider some exports by passing
//! in an [`ExportPredicate`].
//!
//! e.g. for the given JS code:
//!
//! ```js
//! const dog = "dog";
//! const cat = "cat";
//!
//! export const dog = dog;
//! export const catndog = cat + dog;
//! ```
//!
//! The graph would look like this:
//!
//! ```text
//! ╔═══════╗   ┌───┐     
//! ║catndog║──▶│cat│     
//! ╚═══════╝   └───┘     
//!     │                 
//!     ▼                 
//!   ┌───┐     ╔═══╗     
//!   │dog│◀────║dog║     
//!   └───┘     ╚═══╝     
//!                       
//! ── Local              
//! ══ Export                                    
//! ```
//!
//! #### 2: The grouping pass
//!
//! This pass groups declarations into disjoint sets.
//!
//! Starting from the exports, follow all outgoing edges transitively and mark
//! all visited declarations as accessible from this export.
//!
//! Then, starting from the exports again, follow all outgoing edges. When
//! visiting a declaration, move it to the set identified by the set of exports
//! it is accessible from. For instance, if declaration A is accessible from
//! exports B and C, it will go into the set (B, C). However, if the declaration
//! itself is another export, then stop there.
//!
//! ```test
//!  ┌ ─ ─catndog─ ─ ┐     
//!   ╔═══════╗ ┌───┐      
//!  │║catndog║ │cat││     
//!   ╚═══════╝ └───┘      
//!  └ ─ ─ ─ ─ ─ ─ ─ ┘     
//!              │         
//!              ▼         
//!  ┌ dog ┐  ┌(dog)┐      
//!   ╔═══╗    ┌───┐       
//!  │║dog║│─▶││dog││      
//!   ╚═══╝    └───┘       
//!  └ ─ ─ ┘  └ ─ ─ ┘      
//!                        
//! ─x─ Declaration set                                            
//! ```   
//!
//! #### 3: Final pass
//!
//! Given an [`ExportPredicate`], this pass will generate the final module
//! graph, where each declaration set identified in the previous pass will get
//! its own module. Dependencies between these modules are the same as the
//! edges in the graph.
//!
//! Module (dog):
//! ```js
//! export const virtual_dog = "dog";
//! ```
//!
//! Module dog:
//! ```js
//! import { virtual_dog } from "(dog)";
//!
//! export const dog = virtual_dog;
//! ```
//!
//! Module catndog:
//! ```js
//! import { virtual_dog } from "(dog)";
//!
//! const cat = "cat";
//!
//! export const catndog = cat + virtual_dog;
//! ```
//!
//! #### Note
//!
//! For the SSG and SSG-complementary operation, we don't actually need to run
//! the second pass, since we don't need to separate declarations into their
//! own modules. Instead, we will generate three modules:
//! 1. The original module, with all declarations, for initial server-side SSR.
//! 2. The client-side module, with SSG and SSR functions eliminated.
//! 3. The server-side HMR module, with all but SSG and SSR functions
//!    eliminated.

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

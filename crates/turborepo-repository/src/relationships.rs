//! Parser-neutral package relationship vocabulary.
//!
//! Producers use these plain data types without depending on repository
//! validation or package-graph implementation details.

/// The semantic role of a declared package relationship.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyKind {
    Production,
    Optional,
    Development,
    Peer { optional: bool },
}

/// The normalized target of one package relationship.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelationshipTarget {
    Internal(String),
    UnresolvedExternal { name: String, specifier: String },
}

/// One parser-neutral relationship supplied by a package producer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relationship {
    declaration_alias: Option<String>,
    kind: DependencyKind,
    target: RelationshipTarget,
    orders_tasks: bool,
}

impl Relationship {
    pub fn new(
        declaration_name: impl Into<String> + AsRef<str>,
        kind: DependencyKind,
        target: RelationshipTarget,
    ) -> Self {
        let target_name = match &target {
            RelationshipTarget::Internal(target) => target.as_str(),
            RelationshipTarget::UnresolvedExternal { name, .. } => name.as_str(),
        };
        // Only an alias — a declaration name that differs from the resolved
        // target — needs to be owned. When they match (the common case, e.g.
        // every unresolved external dependency) skip the allocation entirely
        // instead of allocating a `String` and immediately discarding it.
        let declaration_alias =
            (declaration_name.as_ref() != target_name).then(|| declaration_name.into());
        Self {
            declaration_alias,
            kind,
            target,
            orders_tasks: true,
        }
    }

    pub fn internal(target: impl Into<String>, kind: DependencyKind) -> Self {
        // The declaration name equals the target here, so there is never an
        // alias to retain. Build directly to avoid cloning the target only to
        // compare it against itself and drop the copy.
        Self {
            declaration_alias: None,
            kind,
            target: RelationshipTarget::Internal(target.into()),
            orders_tasks: true,
        }
    }

    /// Construct an internal relationship that contributes to hashing and
    /// affectedness without adding a task-ordering edge. Cargo uses this for
    /// development relationships that would make its package graph cyclic.
    pub fn internal_input(target: impl Into<String>, kind: DependencyKind) -> Self {
        let mut relationship = Self::internal(target, kind);
        relationship.orders_tasks = false;
        relationship
    }

    pub fn declaration_name(&self) -> &str {
        self.declaration_alias
            .as_deref()
            .unwrap_or_else(|| match &self.target {
                RelationshipTarget::Internal(target) => target,
                RelationshipTarget::UnresolvedExternal { name, .. } => name,
            })
    }

    pub fn kind(&self) -> DependencyKind {
        self.kind
    }

    pub fn target(&self) -> &RelationshipTarget {
        &self.target
    }

    pub fn orders_tasks(&self) -> bool {
        self.orders_tasks
    }
}

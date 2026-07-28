//! Parser-neutral package relationship vocabulary.
//!
//! Producers use these plain data types without depending on repository
//! validation or package-graph implementation details.

/// The semantic role of a declared package relationship.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyKind {
    Production,
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
}

impl Relationship {
    pub fn new(
        declaration_name: impl Into<String>,
        kind: DependencyKind,
        target: RelationshipTarget,
    ) -> Self {
        let declaration_name = declaration_name.into();
        let target_name = match &target {
            RelationshipTarget::Internal(target) => target,
            RelationshipTarget::UnresolvedExternal { name, .. } => name,
        };
        Self {
            declaration_alias: (declaration_name.as_str() != target_name)
                .then_some(declaration_name),
            kind,
            target,
        }
    }

    pub fn internal(target: impl Into<String>, kind: DependencyKind) -> Self {
        let target = target.into();
        Self::new(target.clone(), kind, RelationshipTarget::Internal(target))
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
}

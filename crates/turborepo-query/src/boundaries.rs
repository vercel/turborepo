use turborepo_query_api::BoundaryDiagnostic;

use crate::Diagnostic;

impl From<BoundaryDiagnostic> for Diagnostic {
    fn from(diagnostic: BoundaryDiagnostic) -> Self {
        Self {
            message: diagnostic.message,
            reason: diagnostic.reason,
            path: diagnostic.path,
            import: diagnostic.import,
            start: diagnostic.start,
            end: diagnostic.end,
        }
    }
}

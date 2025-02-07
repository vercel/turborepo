use super::Diagnostic;
use crate::boundaries::BoundariesDiagnostic;

impl From<BoundariesDiagnostic> for Diagnostic {
    fn from(diagnostic: BoundariesDiagnostic) -> Self {
        let message = diagnostic.to_string();
        match diagnostic {
            BoundariesDiagnostic::NotTypeOnlyImport { import, span, text } => Diagnostic {
                message,
                path: Some(text.name().to_string()),
                start: Some(span.offset()),
                end: Some(span.offset() + span.len()),
                import: Some(import),
                reason: None,
            },
            BoundariesDiagnostic::PackageNotFound { name, span, text } => Diagnostic {
                message,
                path: Some(text.name().to_string()),
                start: Some(span.offset()),
                end: Some(span.offset() + span.len()),
                import: Some(name.to_string()),
                reason: None,
            },
            BoundariesDiagnostic::ImportLeavesPackage { import, span, text } => Diagnostic {
                message,
                path: Some(text.name().to_string()),
                start: Some(span.offset()),
                end: Some(span.offset() + span.len()),
                import: Some(import),
                reason: None,
            },
            BoundariesDiagnostic::ParseError(_, _) => Diagnostic {
                message,
                start: None,
                end: None,
                import: None,
                path: None,
                reason: None,
            },

            BoundariesDiagnostic::NoTagInAllowlist {
                source_package_name: _,
                help: _,
                secondary: _,
                package_name,
                span,
                text,
            } => Diagnostic {
                message,
                path: Some(text.name().to_string()),
                start: span.map(|span| span.offset()),
                end: span.map(|span| span.offset() + span.len()),
                import: Some(package_name.to_string()),
                reason: None,
            },
            BoundariesDiagnostic::DeniedTag {
                source_package_name: _,
                secondary: _,
                package_name,
                tag,
                span,
                text,
            } => Diagnostic {
                message,
                path: Some(text.name().to_string()),
                start: span.map(|span| span.offset()),
                end: span.map(|span| span.offset() + span.len()),
                import: Some(package_name.to_string()),
                reason: Some(tag),
            },
        }
    }
}

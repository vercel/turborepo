use miette::{NamedSource, SourceSpan};
use turborepo_repository::package_graph::ROOT_PKG_NAME;

use super::{Error, TurboJson, TOPOLOGICAL_PIPELINE_DELIMITER};
use crate::config::UnnecessaryPackageTaskSyntaxError;

pub type TurboJSONValidation = fn(&TurboJson) -> Vec<Error>;

pub fn validate_no_package_task_syntax(turbo_json: &TurboJson) -> Vec<Error> {
    turbo_json
        .tasks
        .iter()
        .filter(|(task_name, _)| task_name.is_package_task())
        .map(|(task_name, entry)| {
            let (span, text) = entry.span_and_text("turbo.json");
            Error::UnnecessaryPackageTaskSyntax(Box::new(UnnecessaryPackageTaskSyntaxError {
                actual: task_name.to_string(),
                wanted: task_name.task().to_string(),
                span,
                text,
            }))
        })
        .collect()
}

pub fn validate_extends(turbo_json: &TurboJson) -> Vec<Error> {
    match turbo_json.extends.first() {
        Some(package_name) if package_name != ROOT_PKG_NAME || turbo_json.extends.len() > 1 => {
            let (span, text) = turbo_json.extends.span_and_text("turbo.json");
            vec![Error::ExtendFromNonRoot { span, text }]
        }
        None => {
            let path = turbo_json
                .path
                .as_ref()
                .map_or("turbo.json", |p| p.as_ref());

            let (span, text) = match turbo_json.text {
                Some(ref text) => {
                    let len = text.len();
                    let span: SourceSpan = (0, len - 1).into();
                    (Some(span), text.to_string())
                }
                None => (None, String::new()),
            };

            vec![Error::NoExtends {
                span,
                text: NamedSource::new(path, text),
            }]
        }
        _ => vec![],
    }
}

pub fn validate_with_has_no_topo(turbo_json: &TurboJson) -> Vec<Error> {
    turbo_json
        .tasks
        .iter()
        .flat_map(|(_, definition)| {
            definition.with.iter().flatten().filter_map(|with_task| {
                if with_task.starts_with(TOPOLOGICAL_PIPELINE_DELIMITER) {
                    let (span, text) = with_task.span_and_text("turbo.json");
                    Some(Error::InvalidTaskWith { span, text })
                } else {
                    None
                }
            })
        })
        .collect()
}

#[cfg(test)]
mod test {
    use turborepo_errors::Spanned;
    use turborepo_task_id::TaskName;
    use turborepo_unescape::UnescapedString;

    use super::*;
    use crate::turbo_json::{Pipeline, RawTaskDefinition};

    #[test]
    fn test_validate_with_has_no_topo() {
        let turbo_json = TurboJson {
            tasks: Pipeline(
                vec![(
                    TaskName::from("dev"),
                    Spanned::new(RawTaskDefinition {
                        with: Some(vec![Spanned::new(UnescapedString::from("^proxy"))]),
                        ..Default::default()
                    }),
                )]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        };

        let errs = validate_with_has_no_topo(&turbo_json);
        assert_eq!(errs.len(), 1);
        let error = &errs[0];
        assert_eq!(
            error.to_string(),
            "`with` cannot use dependency relationships."
        );
    }
}

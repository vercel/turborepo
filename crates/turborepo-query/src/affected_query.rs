use std::fmt::Write;

/// Selects which affected collection the query returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AffectedQuerySelector {
    Packages,
    Tasks,
}

/// Inputs for constructing an affected-packages or affected-tasks query.
/// Environment-derived refs are supplied by the caller rather than read here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AffectedQueryInput {
    pub selector: AffectedQuerySelector,
    pub base: Option<String>,
    pub head: Option<String>,
    pub scm_base: Option<String>,
    pub scm_head: Option<String>,
    pub package_filters: Vec<String>,
    pub task_filters: Vec<String>,
}

/// Builds a GraphQL query for the selected affected collection.
pub fn build_affected_query(input: &AffectedQueryInput) -> String {
    match input.selector {
        AffectedQuerySelector::Packages => build_affected_packages_query(input),
        AffectedQuerySelector::Tasks => build_affected_tasks_query(input),
    }
}

fn build_affected_packages_query(input: &AffectedQueryInput) -> String {
    let mut query = String::from("{ affectedPackages");
    let mut args = build_ref_args(input);
    push_package_filter(&mut args, &input.package_filters);
    append_arguments(&mut query, args);
    query.push_str(" { items { name path reason { __typename } } length } }");
    query
}

fn build_affected_tasks_query(input: &AffectedQueryInput) -> String {
    let mut query = String::from("{ affectedTasks");
    let mut args = build_ref_args(input);
    if !input.task_filters.is_empty() {
        let task_values: Vec<String> = input
            .task_filters
            .iter()
            .map(|task| format!("\"{}\"", escape_graphql_string(task)))
            .collect();
        args.push(format!("tasks: [{}]", task_values.join(", ")));
    }
    push_package_filter(&mut args, &input.package_filters);
    append_arguments(&mut query, args);
    query.push_str(" { items { name fullName package { name } reason { __typename } } length } }");
    query
}

fn build_ref_args(input: &AffectedQueryInput) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(base) = ref_arg(input.base.as_deref(), input.scm_base.as_deref()) {
        args.push(format!("base: \"{}\"", escape_graphql_string(base)));
    }
    if let Some(head) = ref_arg(input.head.as_deref(), input.scm_head.as_deref()) {
        args.push(format!("head: \"{}\"", escape_graphql_string(head)));
    }
    args
}

fn append_arguments(query: &mut String, args: Vec<String>) {
    if !args.is_empty() {
        let joined = args.join(", ");
        let _ = write!(query, "({joined})");
    }
}

fn ref_arg<'a>(cli_value: Option<&'a str>, env_value: Option<&'a str>) -> Option<&'a str> {
    cli_value.or_else(|| env_value.filter(|value| !value.is_empty()))
}

fn push_package_filter(args: &mut Vec<String>, packages: &[String]) {
    if packages.is_empty() {
        return;
    }

    let filter = if packages.len() == 1 {
        format!(
            "{{ equal: {{ field: NAME, value: \"{}\" }} }}",
            escape_graphql_string(&packages[0])
        )
    } else {
        let predicates: Vec<String> = packages
            .iter()
            .map(|package| {
                format!(
                    "{{ equal: {{ field: NAME, value: \"{}\" }} }}",
                    escape_graphql_string(package)
                )
            })
            .collect();
        format!("{{ or: [{}] }}", predicates.join(", "))
    };
    args.push(format!("filter: {filter}"));
}

/// Escapes a Rust string for use as a GraphQL string literal.
pub fn escape_graphql_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\u{0008}' => escaped.push_str("\\b"),
            '\u{000C}' => escaped.push_str("\\f"),
            character if character.is_control() => {
                let _ = write!(escaped, "\\u{:04X}", character as u32);
            }
            character => escaped.push(character),
        }
    }
    escaped
}

/// Returns the number of affected tasks or packages in a query response.
pub fn affected_result_count(json: &str) -> Option<u64> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    value
        .pointer("/data/affectedTasks/length")
        .or_else(|| value.pointer("/data/affectedPackages/length"))
        .and_then(serde_json::Value::as_u64)
}

/// Interprets an affected query response as a CI exit code:
/// `1` when results were found, `0` when there are no results, or `None` when
/// the response does not contain a valid affected count.
pub fn affected_query_exit_code(json: &str) -> Option<i32> {
    affected_result_count(json).map(|count| i32::from(count > 0))
}

#[cfg(test)]
mod tests {
    use super::{
        AffectedQueryInput, AffectedQuerySelector, affected_query_exit_code, affected_result_count,
        build_affected_query, escape_graphql_string, ref_arg,
    };

    fn affected(
        packages: Option<Vec<&str>>,
        tasks: Option<Vec<&str>>,
        base: Option<&str>,
        head: Option<&str>,
    ) -> AffectedQueryInput {
        AffectedQueryInput {
            selector: if packages.is_some() && tasks.is_none() {
                AffectedQuerySelector::Packages
            } else {
                AffectedQuerySelector::Tasks
            },
            base: base.map(str::to_string),
            head: head.map(str::to_string),
            scm_base: None,
            scm_head: None,
            package_filters: packages
                .unwrap_or_default()
                .into_iter()
                .map(str::to_string)
                .collect(),
            task_filters: tasks
                .unwrap_or_default()
                .into_iter()
                .map(str::to_string)
                .collect(),
        }
    }

    fn query(
        packages: Option<Vec<&str>>,
        tasks: Option<Vec<&str>>,
        base: Option<&str>,
        head: Option<&str>,
    ) -> String {
        build_affected_query(&affected(packages, tasks, base, head))
    }

    #[test]
    fn default_selector_queries_affected_tasks() {
        assert_eq!(
            query(None, None, None, None),
            "{ affectedTasks { items { name fullName package { name } reason { __typename } } \
             length } }"
        );
    }

    #[test]
    fn bare_tasks_flag_returns_all_affected_tasks() {
        assert_eq!(
            query(None, Some(vec![]), None, None),
            "{ affectedTasks { items { name fullName package { name } reason { __typename } } \
             length } }"
        );
    }

    #[test]
    fn task_filters_are_forwarded() {
        let query = query(None, Some(vec!["build", "test"]), None, None);
        assert!(query.contains(r#"tasks: ["build", "test"]"#), "{query}");
    }

    #[test]
    fn root_task_shorthand_is_preserved_as_a_task_filter() {
        let query = query(None, Some(vec!["//#zibble:zonk"]), None, None);
        assert!(query.starts_with("{ affectedTasks"), "{query}");
        assert!(query.contains(r#"tasks: ["//#zibble:zonk"]"#), "{query}");
    }

    #[test]
    fn bare_packages_flag_returns_all_affected_packages() {
        assert_eq!(
            query(Some(vec![]), None, None, None),
            "{ affectedPackages { items { name path reason { __typename } } length } }"
        );
    }

    #[test]
    fn single_package_uses_equal_filter() {
        let query = query(Some(vec!["web"]), None, None, None);
        assert!(query.starts_with("{ affectedPackages"), "{query}");
        assert!(
            query.contains(r#"equal: { field: NAME, value: "web" }"#),
            "{query}"
        );
        assert!(
            !query.contains("or:"),
            "single package should not use or: {query}"
        );
    }

    #[test]
    fn multiple_packages_use_or_filter() {
        let query = query(Some(vec!["web", "docs"]), None, None, None);
        assert!(query.contains("or: ["), "{query}");
        assert!(query.contains(r#"value: "web""#), "{query}");
        assert!(query.contains(r#"value: "docs""#), "{query}");
    }

    #[test]
    fn base_and_head_are_included() {
        let tasks_query = query(None, None, Some("main"), Some("HEAD"));
        assert!(tasks_query.starts_with("{ affectedTasks"), "{tasks_query}");
        assert!(tasks_query.contains(r#"base: "main""#), "{tasks_query}");
        assert!(tasks_query.contains(r#"head: "HEAD""#), "{tasks_query}");

        let packages_query = query(Some(vec![]), None, Some("main"), Some("HEAD"));
        assert!(
            packages_query.starts_with("{ affectedPackages"),
            "{packages_query}"
        );
        assert!(
            packages_query.contains(r#"base: "main""#),
            "{packages_query}"
        );
        assert!(
            packages_query.contains(r#"head: "HEAD""#),
            "{packages_query}"
        );
    }

    #[test]
    fn explicit_refs_override_environment_fallbacks() {
        let mut input = affected(None, None, Some("HEAD"), None);
        input.scm_base = Some("main".to_string());
        input.scm_head = Some("develop".to_string());
        let query = build_affected_query(&input);
        assert!(query.contains(r#"base: "HEAD""#), "{query}");
        assert!(query.contains(r#"head: "develop""#), "{query}");
    }

    #[test]
    fn nonempty_environment_refs_are_fallbacks() {
        let mut input = affected(None, None, None, None);
        input.scm_base = Some("main".to_string());
        input.scm_head = Some("HEAD".to_string());
        let query = build_affected_query(&input);
        assert!(query.contains(r#"base: "main""#), "{query}");
        assert!(query.contains(r#"head: "HEAD""#), "{query}");
    }

    #[test]
    fn empty_environment_refs_are_ignored() {
        let mut input = affected(None, None, None, None);
        input.scm_base = Some(String::new());
        input.scm_head = Some(String::new());
        let query = build_affected_query(&input);
        assert!(!query.contains("base:"), "{query}");
        assert!(!query.contains("head:"), "{query}");
    }

    #[test]
    fn ref_arg_uses_nonempty_environment_when_cli_missing() {
        assert_eq!(ref_arg(None, Some("main")), Some("main"));
        assert_eq!(ref_arg(None, Some("")), None);
    }

    #[test]
    fn ref_arg_prefers_cli_value() {
        assert_eq!(ref_arg(Some("HEAD"), Some("main")), Some("HEAD"));
    }

    #[test]
    fn base_with_quotes_is_escaped() {
        let query = query(None, None, Some(r#"feat/"branch"#), None);
        assert!(
            query.contains(r#"base: "feat/\"branch""#),
            "quotes should be escaped: {query}"
        );
    }

    #[test]
    fn package_with_quotes_is_escaped() {
        let query = query(Some(vec![r#"@scope/"pkg""#]), None, None, None);
        assert!(
            query.contains(r#"value: "@scope/\"pkg\""#),
            "quotes should be escaped: {query}"
        );
    }

    #[test]
    fn task_with_quotes_is_escaped() {
        let query = query(None, Some(vec![r#"build"inject"#]), None, None);
        assert!(
            query.contains(r#""build\"inject""#),
            "quotes should be escaped: {query}"
        );
    }

    #[test]
    fn head_with_backslash_is_escaped() {
        let query = query(None, None, None, Some(r"ref\path"));
        assert!(
            query.contains(r#"head: "ref\\path""#),
            "backslash should be escaped: {query}"
        );
    }

    #[test]
    fn escape_control_characters() {
        assert_eq!(escape_graphql_string("a\nb\r\t\0"), "a\\nb\\r\\t\\u0000");
    }

    #[test]
    fn unicode_is_preserved() {
        assert_eq!(escape_graphql_string("日本語"), "日本語");
    }

    #[test]
    fn combined_package_and_task_filters_query_tasks() {
        let query = query(Some(vec!["web"]), Some(vec!["build"]), None, None);
        assert!(query.starts_with("{ affectedTasks"), "{query}");
        assert!(query.contains(r#"tasks: ["build"]"#), "{query}");
        assert!(
            query.contains(r#"filter: { equal: { field: NAME, value: "web" } }"#),
            "{query}"
        );
    }

    #[test]
    fn bare_tasks_and_package_filters_omit_empty_task_list() {
        let query = query(Some(vec!["web"]), Some(vec![]), None, None);
        assert!(query.starts_with("{ affectedTasks"), "{query}");
        assert!(!query.contains("tasks:"), "{query}");
        assert!(query.contains("filter:"), "{query}");
    }

    #[test]
    fn bare_package_and_task_filters_omit_empty_package_filter() {
        let query = query(Some(vec![]), Some(vec!["build"]), None, None);
        assert!(query.starts_with("{ affectedTasks"), "{query}");
        assert!(query.contains(r#"tasks: ["build"]"#), "{query}");
        assert!(!query.contains("filter:"), "{query}");
    }

    #[test]
    fn affected_result_count_reads_task_and_package_lengths() {
        assert_eq!(
            affected_result_count(
                r#"{"data":{"affectedTasks":{"items":[{"name":"build"}],"length":1}}}"#
            ),
            Some(1)
        );
        assert_eq!(
            affected_result_count(r#"{"data":{"affectedTasks":{"items":[],"length":0}}}"#),
            Some(0)
        );
        assert_eq!(
            affected_result_count(
                r#"{"data":{"affectedPackages":{"items":[{"name":"web"}],"length":2}}}"#
            ),
            Some(2)
        );
        assert_eq!(
            affected_result_count(r#"{"data":{"affectedPackages":{"items":[],"length":0}}}"#),
            Some(0)
        );
    }

    #[test]
    fn affected_result_count_rejects_invalid_responses() {
        assert_eq!(
            affected_result_count(r#"{"errors":[{"message":"failed"}]}"#),
            None
        );
        assert_eq!(affected_result_count(r#"{"data":{}}"#), None);
        assert_eq!(affected_result_count("not json"), None);
        assert_eq!(
            affected_result_count(r#"{"data":{"affectedTasks":{"length":"oops"}}}"#),
            None
        );
        assert_eq!(
            affected_result_count(r#"{"data":{"affectedTasks":{"items":[]}}}"#),
            None
        );
    }

    #[test]
    fn affected_exit_code_is_one_when_results_exist() {
        assert_eq!(
            affected_query_exit_code(r#"{"data":{"affectedTasks":{"length":1}}}"#),
            Some(1)
        );
        assert_eq!(
            affected_query_exit_code(r#"{"data":{"affectedPackages":{"length":2}}}"#),
            Some(1)
        );
    }

    #[test]
    fn affected_exit_code_is_zero_when_no_results_exist() {
        assert_eq!(
            affected_query_exit_code(r#"{"data":{"affectedTasks":{"length":0}}}"#),
            Some(0)
        );
    }

    #[test]
    fn affected_exit_code_is_unknown_for_invalid_results() {
        assert_eq!(affected_query_exit_code(r#"{"data":{}}"#), None);
    }
}

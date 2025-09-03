//! Environment variable filtering for tasks and hashing for cache keys.

#![deny(clippy::all)]

use std::{
    collections::HashMap,
    env,
    ops::{Deref, DerefMut},
};

use regex::RegexBuilder;
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

pub mod platform;

const DEFAULT_ENV_VARS: &[&str] = ["VERCEL_ANALYTICS_ID", "VERCEL_TARGET_ENV"].as_slice();

#[derive(Clone, Debug, Error)]
pub enum Error {
    #[error("Failed to parse regex: {0}")]
    Regex(#[from] regex::Error),
}

// TODO: Consider using immutable data structures here
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(transparent)]
pub struct EnvironmentVariableMap(HashMap<String, String>);

impl EnvironmentVariableMap {
    // Returns a deterministically sorted set of EnvironmentVariablePairs
    // from an EnvironmentVariableMap.
    // This is the value that is used upstream as a task hash input,
    // so we need it to be deterministic
    pub fn to_hashable(&self) -> EnvironmentVariablePairs {
        let mut list: Vec<_> = self.iter().map(|(k, v)| format!("{k}={v}")).collect();
        list.sort();

        list
    }

    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<_> = self.keys().cloned().collect();
        names.sort();

        names
    }

    // Returns a deterministically sorted set of EnvironmentVariablePairs
    // from an EnvironmentVariableMap
    // This is the value used to print out the task hash input,
    // so the values are cryptographically hashed
    pub fn to_secret_hashable(&self) -> EnvironmentVariablePairs {
        let mut pairs: Vec<String> = self
            .iter()
            .map(|(k, v)| {
                if !v.is_empty() {
                    let mut hasher = Sha256::new();
                    hasher.update(v.as_bytes());
                    let hash = hasher.finalize();
                    let hexed_hash = hex::encode(hash);
                    format!("{k}={hexed_hash}")
                } else {
                    format!("{k}=")
                }
            })
            .collect();
        // Make it deterministic to facilitate comparisons
        pairs.sort();
        pairs
    }
}

// BySource contains a map of environment variables broken down by the source
#[derive(Debug, Default, Serialize, Clone)]
pub struct BySource {
    pub explicit: EnvironmentVariableMap,
    pub matching: EnvironmentVariableMap,
}

// DetailedMap contains the composite and the detailed maps of environment
// variables All is used as a taskhash input (taskhash.CalculateTaskHash)
// BySource is used by dry runs and run summaries
#[derive(Debug, Default, Serialize, Clone)]
pub struct DetailedMap {
    pub all: EnvironmentVariableMap,
    pub by_source: BySource,
}

// A list of "k=v" strings for env variables and their values
pub type EnvironmentVariablePairs = Vec<String>;

// WildcardMaps is a pair of EnvironmentVariableMaps.
#[derive(Debug)]
pub struct WildcardMaps {
    pub inclusions: EnvironmentVariableMap,
    pub exclusions: EnvironmentVariableMap,
}

impl WildcardMaps {
    // Resolve collapses a WildcardSet into a single EnvironmentVariableMap.
    fn resolve(self) -> EnvironmentVariableMap {
        let mut output = self.inclusions;
        output.difference(&self.exclusions);
        output
    }
}

impl From<HashMap<String, String>> for EnvironmentVariableMap {
    fn from(map: HashMap<String, String>) -> Self {
        EnvironmentVariableMap(map)
    }
}

impl Deref for EnvironmentVariableMap {
    type Target = HashMap<String, String>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for EnvironmentVariableMap {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl EnvironmentVariableMap {
    pub fn infer() -> Self {
        EnvironmentVariableMap(env::vars().collect())
    }

    pub fn into_inner(self) -> HashMap<String, String> {
        self.0
    }

    // Takes another EnvironmentVariableMap and adds it into `self`
    // Overwrites values if they already exist.
    pub fn union(&mut self, another: &EnvironmentVariableMap) {
        for (key, value) in &another.0 {
            self.0.insert(key.clone(), value.clone());
        }
    }

    // Takes another EnvironmentVariableMap and removes matching keys
    // from `self`
    pub fn difference(&mut self, another: &EnvironmentVariableMap) {
        for key in another.0.keys() {
            self.0.remove(key);
        }
    }

    // returns a WildcardMaps after processing wildcards against it.
    fn wildcard_map_from_wildcards(
        &self,
        wildcard_patterns: &[impl AsRef<str>],
    ) -> Result<WildcardMaps, Error> {
        let mut output = WildcardMaps {
            inclusions: EnvironmentVariableMap::default(),
            exclusions: EnvironmentVariableMap::default(),
        };

        let mut include_patterns = Vec::new();
        let mut exclude_patterns = Vec::new();

        for wildcard_pattern in wildcard_patterns {
            let wildcard_pattern = wildcard_pattern.as_ref();
            if let Some(rest) = wildcard_pattern.strip_prefix('!') {
                let exclude_pattern = wildcard_to_regex_pattern(rest);
                exclude_patterns.push(exclude_pattern);
            } else if wildcard_pattern.starts_with("\\!") {
                let include_pattern = wildcard_to_regex_pattern(&wildcard_pattern[1..]);
                include_patterns.push(include_pattern);
            } else {
                let include_pattern = wildcard_to_regex_pattern(wildcard_pattern);
                include_patterns.push(include_pattern);
            }
        }

        let include_regex_string = format!("^({})$", include_patterns.join("|"));
        let exclude_regex_string = format!("^({})$", exclude_patterns.join("|"));

        let case_insensitive = cfg!(windows);
        let include_regex = RegexBuilder::new(&include_regex_string)
            .case_insensitive(case_insensitive)
            .build()?;
        let exclude_regex = RegexBuilder::new(&exclude_regex_string)
            .case_insensitive(case_insensitive)
            .build()?;
        for (env_var, env_value) in &self.0 {
            if !include_patterns.is_empty() && include_regex.is_match(env_var) {
                output.inclusions.insert(env_var.clone(), env_value.clone());
            }
            if !exclude_patterns.is_empty() && exclude_regex.is_match(env_var) {
                output.exclusions.insert(env_var.clone(), env_value.clone());
            }
        }

        Ok(output)
    }

    // Returns an EnvironmentVariableMap containing the variables
    // in the environment which match an array of wildcard patterns.
    pub fn from_wildcards(
        &self,
        wildcard_patterns: &[impl AsRef<str>],
    ) -> Result<EnvironmentVariableMap, Error> {
        if wildcard_patterns.is_empty() {
            return Ok(EnvironmentVariableMap::default());
        }

        let resolved_set = self.wildcard_map_from_wildcards(wildcard_patterns)?;
        Ok(resolved_set.resolve())
    }

    // FromWildcardsUnresolved returns a wildcardSet specifying the inclusions and
    // exclusions discovered from a set of wildcard patterns. This is used to ensure
    // that user exclusions have primacy over inferred inclusions.
    pub fn wildcard_map_from_wildcards_unresolved(
        &self,
        wildcard_patterns: &[impl AsRef<str>],
    ) -> Result<WildcardMaps, Error> {
        if wildcard_patterns.is_empty() {
            return Ok(WildcardMaps {
                inclusions: EnvironmentVariableMap::default(),
                exclusions: EnvironmentVariableMap::default(),
            });
        }

        self.wildcard_map_from_wildcards(wildcard_patterns)
    }

    /// Return a detailed map for which environment variables are factored into
    /// the task's hash
    pub fn hashable_task_env(
        &self,
        computed_wildcards: &[String],
        task_env: &[String],
    ) -> Result<DetailedMap, Error> {
        let mut explicit_env_var_map = EnvironmentVariableMap::default();
        let mut all_env_var_map = EnvironmentVariableMap::default();
        let mut matching_env_var_map = EnvironmentVariableMap::default();
        let inference_env_var_map = self.from_wildcards(computed_wildcards)?;

        let user_env_var_set = self.wildcard_map_from_wildcards_unresolved(task_env)?;

        all_env_var_map.union(&user_env_var_set.inclusions);
        all_env_var_map.union(&inference_env_var_map);
        all_env_var_map.difference(&user_env_var_set.exclusions);

        explicit_env_var_map.union(&user_env_var_set.inclusions);
        explicit_env_var_map.difference(&user_env_var_set.exclusions);

        matching_env_var_map.union(&inference_env_var_map);
        matching_env_var_map.difference(&user_env_var_set.exclusions);

        Ok(DetailedMap {
            all: all_env_var_map,
            by_source: BySource {
                explicit: explicit_env_var_map,
                matching: matching_env_var_map,
            },
        })
    }

    /// Constructs an environment map that contains pass through environment
    /// variables
    pub fn pass_through_env(
        &self,
        builtins: &[&str],
        global_env: &Self,
        task_pass_through: &[impl AsRef<str>],
    ) -> Result<Self, Error> {
        let mut pass_through_env = EnvironmentVariableMap::default();
        let default_env_var_pass_through_map = self.from_wildcards(builtins)?;
        let task_pass_through_env =
            self.wildcard_map_from_wildcards_unresolved(task_pass_through)?;

        pass_through_env.union(&default_env_var_pass_through_map);
        pass_through_env.union(global_env);
        pass_through_env.union(&task_pass_through_env.inclusions);
        pass_through_env.difference(&task_pass_through_env.exclusions);

        Ok(pass_through_env)
    }
}

const WILDCARD: char = '*';
const WILDCARD_ESCAPE: char = '\\';
const REGEX_WILDCARD_SEGMENT: &str = ".*";

fn wildcard_to_regex_pattern(pattern: &str) -> String {
    let mut regex_string = Vec::new();
    let mut previous_index = 0;
    let mut previous_char: Option<char> = None;

    for (i, char) in pattern.char_indices() {
        if char == WILDCARD {
            if previous_char == Some(WILDCARD_ESCAPE) {
                // Found a literal *
                // Replace the trailing "\*" with just "*" before adding the segment.
                regex_string.push(regex::escape(&format!(
                    "{}*",
                    &pattern[previous_index..(i - 1)]
                )));
            } else {
                // Found a wildcard
                // Add in the static segment since the last wildcard. Can be zero length.
                regex_string.push(regex::escape(&pattern[previous_index..i]));

                // Add a dynamic segment if it isn't adjacent to another dynamic segment.
                if let Some(last_segment) = regex_string.last()
                    && last_segment != REGEX_WILDCARD_SEGMENT
                {
                    regex_string.push(REGEX_WILDCARD_SEGMENT.to_string());
                }
            }

            // Advance the pointer.
            previous_index = i + 1;
        }
        previous_char = Some(char);
    }

    // Add the last static segment. Can be zero length.
    regex_string.push(regex::escape(&pattern[previous_index..]));

    regex_string.join("")
}

pub fn get_global_hashable_env_vars(
    env_at_execution_start: &EnvironmentVariableMap,
    global_env: &[String],
) -> Result<DetailedMap, Error> {
    let default_env_var_map = env_at_execution_start.from_wildcards(DEFAULT_ENV_VARS)?;

    let user_env_var_set =
        env_at_execution_start.wildcard_map_from_wildcards_unresolved(global_env)?;

    let mut all_env_var_map = EnvironmentVariableMap::default();
    all_env_var_map.union(&user_env_var_set.inclusions);
    all_env_var_map.union(&default_env_var_map);
    all_env_var_map.difference(&user_env_var_set.exclusions);

    let mut explicit_env_var_map = EnvironmentVariableMap::default();
    explicit_env_var_map.union(&user_env_var_set.inclusions);
    explicit_env_var_map.difference(&user_env_var_set.exclusions);

    let mut matching_env_var_map = EnvironmentVariableMap::default();
    matching_env_var_map.union(&default_env_var_map);
    matching_env_var_map.difference(&explicit_env_var_map);

    Ok(DetailedMap {
        all: all_env_var_map,
        by_source: BySource {
            explicit: explicit_env_var_map,
            matching: matching_env_var_map,
        },
    })
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use super::*;

    #[test_case("LITERAL_\\*", "LITERAL_\\*" ; "literal star")]
    #[test_case("\\*LEADING", "\\*LEADING" ; "leading literal star")]
    #[test_case("\\!LEADING", "\\\\!LEADING" ; "leading literal bang")]
    #[test_case("!LEADING", "!LEADING" ; "leading bang")]
    #[test_case("*LEADING", ".*LEADING" ; "leading star")]
    fn test_wildcard_to_regex_pattern(pattern: &str, expected: &str) {
        let actual = super::wildcard_to_regex_pattern(pattern);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_case_sensitivity() {
        let start = EnvironmentVariableMap(
            vec![("Turbo".to_string(), "true".to_string())]
                .into_iter()
                .collect(),
        );
        let actual = start.from_wildcards(&["TURBO"]).unwrap();
        if cfg!(windows) {
            assert_eq!(actual.get("Turbo").map(|s| s.as_str()), Some("true"));
        } else {
            assert_eq!(actual.get("Turbo"), None);
        }
    }

    #[test_case(&[], &["VERCEL_ANALYTICS_ID", "VERCEL_TARGET_ENV"] ; "defaults")]
    #[test_case(&["!VERCEL*"], &[] ; "removing defaults")]
    #[test_case(&["FOO*", "!FOOD"], &["FOO", "FOOBAR", "VERCEL_ANALYTICS_ID", "VERCEL_TARGET_ENV"] ; "intersecting globs")]
    fn test_global_env(inputs: &[&str], expected: &[&str]) {
        let env_at_start = EnvironmentVariableMap(
            vec![
                ("VERCEL_TARGET_ENV", "prod"),
                ("VERCEL_ANALYTICS_ID", "1"),
                ("FOO", "bar"),
                ("FOOBAR", "baz"),
                ("FOOD", "cheese"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
        );
        let inputs = inputs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let actual = get_global_hashable_env_vars(&env_at_start, &inputs).unwrap();
        let mut actual = actual.all.keys().map(|s| s.as_str()).collect::<Vec<_>>();
        actual.sort();
        assert_eq!(actual, expected);
    }

    #[test_case(&["FOO*"], &["BAR"], &["BAR", "FOO", "FOOBAR", "FOOD"] ; "wildcard")]
    #[test_case(&["FOO*", "!FOOBAR"], &["BAR"], &["BAR", "FOO", "FOOD"] ; "omit wild")]
    #[test_case(&["FOO*"], &["!FOOBAR"], &["FOO", "FOOD"] ; "omit task")]
    fn test_hashable_env(wildcards: &[&str], task: &[&str], expected: &[&str]) {
        let env_at_start = EnvironmentVariableMap(
            vec![
                ("FOO", "bar"),
                ("FOOBAR", "baz"),
                ("FOOD", "cheese"),
                ("BAR", "nuts"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
        );
        let wildcards: Vec<_> = wildcards.iter().map(|s| s.to_string()).collect();
        let task: Vec<_> = task.iter().map(|s| s.to_string()).collect();
        let output = env_at_start.hashable_task_env(&wildcards, &task).unwrap();
        let mut actual: Vec<_> = output.all.keys().map(|s| s.as_str()).collect();
        actual.sort();
        assert_eq!(actual, expected);
    }

    #[test_case(&["FOO*"], &["FOO", "FOOBAR", "FOOD", "PATH"] ; "folds 3 sources")]
    #[test_case(&["!FOO"], &["PATH"] ; "remove global")]
    #[test_case(&["!PATH"], &["FOO"] ; "remove builtin")]
    #[test_case(&["FOO*", "!FOOD"], &["FOO", "FOOBAR", "PATH"] ; "mixing negations")]
    fn test_pass_through_env(task: &[&str], expected: &[&str]) {
        let env_at_start = EnvironmentVariableMap(
            vec![
                ("PATH", "of"),
                ("FOO", "bar"),
                ("FOOBAR", "baz"),
                ("FOOD", "cheese"),
                ("BAR", "nuts"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
        );
        let global_env = EnvironmentVariableMap(
            vec![("FOO", "bar")]
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect(),
        );
        let output = env_at_start
            .pass_through_env(&["PATH"], &global_env, task)
            .unwrap();
        let mut actual: Vec<_> = output.keys().map(|s| s.as_str()).collect();
        actual.sort();
        assert_eq!(actual, expected);
    }
}

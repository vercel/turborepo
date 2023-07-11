#![deny(clippy::all)]

use std::{
    collections::HashMap,
    env,
    ops::{Deref, DerefMut},
    string::ToString,
};

use regex::Regex;
use serde::Serialize;
use thiserror::Error;

const DEFAULT_ENV_VARS: [&str; 1] = ["VERCEL_ANALYTICS_ID"];

#[derive(Clone, Debug, Error)]
pub enum Error {
    #[error("Failed to parse regex: {0}")]
    Regex(#[from] regex::Error),
}

// TODO: Consider using immutable data structures here
#[derive(Clone, Debug, Default, Serialize)]
#[serde(transparent)]
pub struct EnvironmentVariableMap(HashMap<String, String>);

impl EnvironmentVariableMap {
    pub fn to_hashable(&self) -> EnvironmentVariablePairs {
        self.iter().map(|(k, v)| format!("{}={}", k, v)).collect()
    }

    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<_> = self.keys().cloned().collect();
        names.sort();

        names
    }
}

// BySource contains a map of environment variables broken down by the source
#[derive(Debug, Serialize)]
pub struct BySource {
    pub explicit: EnvironmentVariableMap,
    pub matching: EnvironmentVariableMap,
}

// DetailedMap contains the composite and the detailed maps of environment
// variables All is used as a taskhash input (taskhash.CalculateTaskHash)
// BySource is used by dry runs and run summaries
#[derive(Debug, Serialize)]
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
    ) -> Result<WildcardMaps, regex::Error> {
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

        let include_regex = Regex::new(&include_regex_string)?;
        let exclude_regex = Regex::new(&exclude_regex_string)?;
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
    ) -> Result<EnvironmentVariableMap, regex::Error> {
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
        wildcard_patterns: &[String],
    ) -> Result<WildcardMaps, regex::Error> {
        if wildcard_patterns.is_empty() {
            return Ok(WildcardMaps {
                inclusions: EnvironmentVariableMap::default(),
                exclusions: EnvironmentVariableMap::default(),
            });
        }

        self.wildcard_map_from_wildcards(wildcard_patterns)
    }
}

const WILDCARD: char = '*';
const WILDCARD_ESCAPE: char = '\\';
const REGEX_WILDCARD_SEGMENT: &str = ".*";

fn wildcard_to_regex_pattern(pattern: &str) -> String {
    let mut regex_string = Vec::new();
    let mut previous_index = 0;
    let mut previous_char: Option<char> = None;

    for (i, char) in pattern.chars().enumerate() {
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
                if let Some(last_segment) = regex_string.last() {
                    if last_segment != REGEX_WILDCARD_SEGMENT {
                        regex_string.push(REGEX_WILDCARD_SEGMENT.to_string());
                    }
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
    let default_env_var_map = env_at_execution_start.from_wildcards(&DEFAULT_ENV_VARS[..])?;

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

    #[test_case("LITERAL_\\*", "LITERAL_\\*" ; "literal star")]
    #[test_case("\\*LEADING", "\\*LEADING" ; "leading literal star")]
    #[test_case("\\!LEADING", "\\\\!LEADING" ; "leading literal bang")]
    #[test_case("!LEADING", "!LEADING" ; "leading bang")]
    #[test_case("*LEADING", ".*LEADING" ; "leading star")]
    fn test_wildcard_to_regex_pattern(pattern: &str, expected: &str) {
        let actual = super::wildcard_to_regex_pattern(pattern);
        assert_eq!(actual, expected);
    }
}

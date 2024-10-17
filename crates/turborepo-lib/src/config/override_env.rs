use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
};

use super::{env::truth_env_var, ConfigurationOptions, Error, ResolvedConfigurationOptions};
use crate::turbo_json::UIMode;

/*
Hi! If you're new here:
1. The general pattern is that:
  - ConfigurationOptions.token corresponds to TURBO_TOKEN or VERCEL_ARTIFACTS_TOKEN
  - ConfigurationOptions.team_id corresponds to TURBO_TEAMID or VERCEL_ARTIFACTS_OWNER
  - ConfigurationOptions.team_slug corresponds to TURBO_TEAM
1. We're ultimately poking around the env vars looking for _pairs_ that make sense.
   Since we presume that users are the only ones sending TURBO_* and Vercel is the only one sending VERCEL_*, we can make some assumptions.  Namely, we assume that if we have one of VERCEL_ARTIFACTS_OWNER or VERCEL_ARTIFACTS_TOKEN we will always have both.
1. Watch out for mixing up `TURBO_TEAM` and `TURBO_TEAMID`.  Same for ConfigurationOptions.team_id and ConfigurationOptions.team_slug.
*/

/// these correspond directly to the environment variables that this module
/// needs to do it's work
#[allow(non_snake_case)]
#[derive(Default)]
struct Input {
    TURBO_TEAM: Option<String>,
    TURBO_TEAMID: Option<String>,
    TURBO_TOKEN: Option<String>,
    VERCEL_ARTIFACTS_OWNER: Option<String>,
    VERCEL_ARTIFACTS_TOKEN: Option<String>,
}

impl Input {
    fn new() -> Self {
        Self::default()
    }
}

impl<'a> From<&'a HashMap<OsString, OsString>> for Input {
    fn from(environment: &'a HashMap<OsString, OsString>) -> Self {
        Self {
            TURBO_TEAM: environment
                .get(OsStr::new("turbo_team"))
                .map(|s| s.to_str().unwrap().to_string()),
            TURBO_TEAMID: environment
                .get(OsStr::new("turbo_teamid"))
                .map(|s| s.to_str().unwrap().to_string()),
            TURBO_TOKEN: environment
                .get(OsStr::new("turbo_token"))
                .map(|s| s.to_str().unwrap().to_string()),
            VERCEL_ARTIFACTS_OWNER: environment
                .get(OsStr::new("vercel_artifacts_owner"))
                .map(|s| s.to_str().unwrap().to_string()),
            VERCEL_ARTIFACTS_TOKEN: environment
                .get(OsStr::new("vercel_artifacts_token"))
                .map(|s| s.to_str().unwrap().to_string()),
        }
    }
}

// this is an internal structure (that's a partial of ConfigurationOptions) that
// we use to store
struct Output {
    /// maps to ConfigurationOptions.team_id
    team_id: Option<String>,
    // maps to ConfigurationOptions.team_slug
    team_slug: Option<String>,
    // maps to ConfigurationOptions.token
    token: Option<String>,
}

impl Output {
    fn new() -> Self {
        Self {
            team_id: None,
            team_slug: None,
            token: None,
        }
    }
}

// get Output from Input
impl From<Input> for Output {
    fn from(input: Input) -> Self {
        let mut output = Output::new();

        // TURBO_TEAMID+TURBO_TOKEN
        if input.TURBO_TEAMID.is_some() && input.TURBO_TOKEN.is_some() {
            output.team_id = input.TURBO_TEAMID;
            output.token = input.TURBO_TOKEN;

            if input.TURBO_TEAM.is_some() {
                // there can also be a TURBO_TEAM, so we'll use that as well
                output.team_slug = input.TURBO_TEAM;
            }

            return output;
        }

        // TURBO_TEAM+TURBO_TOKEN
        if input.TURBO_TEAM.is_some() && input.TURBO_TOKEN.is_some() {
            output.team_slug = input.TURBO_TEAM;
            output.token = input.TURBO_TOKEN;

            if input.TURBO_TEAMID.is_some() {
                // there can also be a TURBO_TEAMID, so we'll use that as well
                output.team_id = input.TURBO_TEAMID;
            }

            return output;
        }

        // if there's both Vercel items, we use those next
        if input.VERCEL_ARTIFACTS_OWNER.is_some() && input.VERCEL_ARTIFACTS_TOKEN.is_some() {
            output.team_id = input.VERCEL_ARTIFACTS_OWNER;
            output.token = input.VERCEL_ARTIFACTS_TOKEN;
            return output;
        }

        // from this point below, there's no token we can do anything with
        // ------------------------------------------------

        // if there's no token, this is also permissible
        if input.TURBO_TEAMID.is_some() && input.TURBO_TEAM.is_some() {
            output.team_id = input.TURBO_TEAMID;
            output.team_slug = input.TURBO_TEAM;
            return output;
        }

        // handle "only" cases
        // ------------------------------------------------
        if input.TURBO_TEAMID.is_some() {
            output.team_id = input.TURBO_TEAMID;
            return output;
        }

        if input.TURBO_TEAM.is_some() {
            output.team_slug = input.TURBO_TEAM;

            if input.VERCEL_ARTIFACTS_OWNER.is_some() {
                output.team_id = input.VERCEL_ARTIFACTS_OWNER;
            }

            return output;
        }

        if input.VERCEL_ARTIFACTS_OWNER.is_some() {
            output.team_id = input.VERCEL_ARTIFACTS_OWNER;
            return output;
        }

        Output::new()
    }
}

pub struct OverrideEnvVars<'a> {
    environment: &'a HashMap<OsString, OsString>,
    output: Output,
}

impl<'a> OverrideEnvVars<'a> {
    pub fn new(environment: &'a HashMap<OsString, OsString>) -> Result<Self, Error> {
        let input = Input::from(environment);
        let output = Output::from(input);

        Ok(Self {
            environment,
            output,
        })
    }

    fn ui(&self) -> Option<UIMode> {
        // TODO double check on what's going on here
        let value = self
            .environment
            .get(OsStr::new("ci"))
            .or_else(|| self.environment.get(OsStr::new("no_color")))?;
        match truth_env_var(value.to_str()?)? {
            true => Some(UIMode::Stream),
            false => None,
        }
    }
}

impl<'a> ResolvedConfigurationOptions for OverrideEnvVars<'a> {
    fn get_configuration_options(
        &self,
        _existing_config: &ConfigurationOptions,
    ) -> Result<ConfigurationOptions, Error> {
        let output = ConfigurationOptions {
            team_id: self.output.team_id.clone(),
            token: self.output.token.clone(),
            team_slug: self.output.team_slug.clone(),
            ui: self.ui(),
            ..Default::default()
        };
        Ok(output)
    }
}

#[cfg(test)]
mod test {
    use lazy_static::lazy_static;

    use super::*;

    lazy_static! {
        static ref VERCEL_ARTIFACTS_OWNER: String = String::from("valueof:VERCEL_ARTIFACTS_OWNER");
        static ref VERCEL_ARTIFACTS_TOKEN: String = String::from("valueof:VERCEL_ARTIFACTS_TOKEN");
        static ref TURBO_TEAMID: String = String::from("valueof:TURBO_TEAMID");
        static ref TURBO_TEAM: String = String::from("valueof:TURBO_TEAM");
        static ref TURBO_TOKEN: String = String::from("valueof:TURBO_TOKEN");
    }

    struct TestCase {
        input: Input,
        output: Output,
        reason: &'static str,
    }

    impl TestCase {
        fn new() -> Self {
            Self {
                input: Input::new(),
                output: Output::new(),
                reason: "missing",
            }
        }

        fn reason(mut self, reason: &'static str) -> Self {
            self.reason = reason;
            self
        }

        #[allow(non_snake_case)]
        fn VERCEL_ARTIFACTS_OWNER(mut self) -> Self {
            self.input.VERCEL_ARTIFACTS_OWNER = Some(VERCEL_ARTIFACTS_OWNER.clone());
            self
        }

        #[allow(non_snake_case)]
        fn VERCEL_ARTIFACTS_TOKEN(mut self) -> Self {
            self.input.VERCEL_ARTIFACTS_TOKEN = Some(VERCEL_ARTIFACTS_TOKEN.clone());
            self
        }

        #[allow(non_snake_case)]
        fn TURBO_TEAMID(mut self) -> Self {
            self.input.TURBO_TEAMID = Some(TURBO_TEAMID.clone());
            self
        }

        #[allow(non_snake_case)]
        fn TURBO_TEAM(mut self) -> Self {
            self.input.TURBO_TEAM = Some(TURBO_TEAM.clone());
            self
        }

        #[allow(non_snake_case)]
        fn TURBO_TOKEN(mut self) -> Self {
            self.input.TURBO_TOKEN = Some(TURBO_TOKEN.clone());
            self
        }

        fn team_id(mut self, value: String) -> Self {
            self.output.team_id = Some(value);
            self
        }

        fn team_slug(mut self, value: String) -> Self {
            self.output.team_slug = Some(value);
            self
        }

        fn token(mut self, value: String) -> Self {
            self.output.token = Some(value);
            self
        }
    }

    #[test]
    fn test_all_the_combos() {
        let cases: &[TestCase] = &[
            //
            // Get nothing back
            // ------------------------------
            TestCase::new().reason("no env vars set"),
            TestCase::new()
                .reason("just VERCEL_ARTIFACTS_TOKEN")
                .VERCEL_ARTIFACTS_TOKEN(),
            TestCase::new().reason("just TURBO_TOKEN").TURBO_TOKEN(),
            //
            // When 3rd Party Wins with all three
            // ------------------------------
            TestCase::new()
                .reason("we can use all of TURBO_TEAM, TURBO_TEAMID, and TURBO_TOKEN")
                .TURBO_TEAM()
                .TURBO_TEAMID()
                .TURBO_TOKEN()
                .team_id(TURBO_TEAMID.clone())
                .team_slug(TURBO_TEAM.clone())
                .token(TURBO_TOKEN.clone()),
            TestCase::new()
                .reason("if we have a 3rd party trifecta, that wins, even against a Vercel Pair")
                .TURBO_TEAM()
                .TURBO_TEAMID()
                .TURBO_TOKEN()
                .VERCEL_ARTIFACTS_OWNER()
                .VERCEL_ARTIFACTS_TOKEN()
                .team_id(TURBO_TEAMID.clone())
                .team_slug(TURBO_TEAM.clone())
                .token(TURBO_TOKEN.clone()),
            TestCase::new()
                .reason("a 3rd party trifecta wins against a partial Vercel (just artifacts token)")
                .TURBO_TEAM()
                .TURBO_TEAMID()
                .TURBO_TOKEN()
                .VERCEL_ARTIFACTS_TOKEN()
                .team_id(TURBO_TEAMID.clone())
                .team_slug(TURBO_TEAM.clone())
                .token(TURBO_TOKEN.clone()),
            TestCase::new()
                .reason("a 3rd party trifecta wins against a partial Vercel (just artifacts owner)")
                .TURBO_TEAM()
                .TURBO_TEAMID()
                .TURBO_TOKEN()
                .VERCEL_ARTIFACTS_OWNER()
                .team_id(TURBO_TEAMID.clone())
                .team_slug(TURBO_TEAM.clone())
                .token(TURBO_TOKEN.clone()),
            //
            // When 3rd Party Wins with team_slug
            // ------------------------------
            TestCase::new()
                .reason("golden path for 3rd party, not deployed on Vercel")
                .TURBO_TEAM()
                .TURBO_TOKEN()
                .team_slug(TURBO_TEAM.clone())
                .token(TURBO_TOKEN.clone()),
            TestCase::new()
                .reason(
                    "a TURBO_TEAM+TURBO_TOKEN pair wins against an incomplete Vercel (just \
                     artifacts token)",
                )
                .TURBO_TEAM()
                .TURBO_TOKEN()
                .VERCEL_ARTIFACTS_TOKEN() // disregarded
                .team_slug(TURBO_TEAM.clone())
                .token(TURBO_TOKEN.clone()),
            TestCase::new()
                .reason("golden path for 3rd party, deployed on Vercel")
                .TURBO_TEAM()
                .TURBO_TOKEN()
                .VERCEL_ARTIFACTS_OWNER() // normally this would map to team_id, but not with a complete 3rd party pair
                .VERCEL_ARTIFACTS_TOKEN()
                .team_slug(TURBO_TEAM.clone())
                .token(TURBO_TOKEN.clone()),
            //
            // When 3rd Party Wins with team_id
            // ------------------------------
            TestCase::new()
                .reason("if they pass a TURBO_TEAMID and a TURBO_TOKEN, we use them")
                .TURBO_TEAMID()
                .TURBO_TOKEN()
                .team_id(TURBO_TEAMID.clone())
                .token(TURBO_TOKEN.clone()),
            TestCase::new()
                .reason("a TURBO_TEAMID+TURBO_TOKEN pair will also win against a Vercel pair")
                .TURBO_TEAMID()
                .TURBO_TOKEN()
                .VERCEL_ARTIFACTS_OWNER()
                .VERCEL_ARTIFACTS_TOKEN()
                .team_id(TURBO_TEAMID.clone())
                .token(TURBO_TOKEN.clone()),
            TestCase::new()
                .reason(
                    "a TURBO_TEAMID+TURBO_TOKEN pair wins against an incomplete Vercel (just \
                     artifacts token)",
                )
                .TURBO_TEAMID()
                .TURBO_TOKEN()
                .VERCEL_ARTIFACTS_TOKEN()
                .team_id(TURBO_TEAMID.clone())
                .token(TURBO_TOKEN.clone()),
            //
            // When Vercel Wins
            // ------------------------------
            TestCase::new()
                .reason("golden path on Vercel zero config")
                .VERCEL_ARTIFACTS_OWNER()
                .VERCEL_ARTIFACTS_TOKEN()
                .team_id(VERCEL_ARTIFACTS_OWNER.clone())
                .token(VERCEL_ARTIFACTS_TOKEN.clone()),
            TestCase::new()
                .reason("Vercel wins: disregard just TURBO_TOKEN")
                .TURBO_TOKEN()
                .VERCEL_ARTIFACTS_OWNER()
                .VERCEL_ARTIFACTS_TOKEN()
                .team_id(VERCEL_ARTIFACTS_OWNER.clone())
                .token(VERCEL_ARTIFACTS_TOKEN.clone()),
            TestCase::new()
                .reason("Vercel wins: disregard just TURBO_TEAM")
                .TURBO_TEAM()
                .VERCEL_ARTIFACTS_OWNER()
                .VERCEL_ARTIFACTS_TOKEN()
                .team_id(VERCEL_ARTIFACTS_OWNER.clone())
                .token(VERCEL_ARTIFACTS_TOKEN.clone()),
            TestCase::new()
                .reason("Vercel wins: disregard just TURBO_TEAMID")
                .TURBO_TEAMID()
                .VERCEL_ARTIFACTS_OWNER()
                .VERCEL_ARTIFACTS_TOKEN()
                .team_id(VERCEL_ARTIFACTS_OWNER.clone())
                .token(VERCEL_ARTIFACTS_TOKEN.clone()),
            TestCase::new()
                .reason("Vercel wins if TURBO_TOKEN is missing")
                .TURBO_TEAM()
                .TURBO_TEAMID()
                .VERCEL_ARTIFACTS_OWNER()
                .VERCEL_ARTIFACTS_TOKEN()
                .team_id(VERCEL_ARTIFACTS_OWNER.clone())
                .token(VERCEL_ARTIFACTS_TOKEN.clone()),
            //
            // Just get a team_id
            // ------------------------------
            TestCase::new()
                .reason("just VERCEL_ARTIFACTS_OWNER")
                .VERCEL_ARTIFACTS_OWNER()
                .team_id(VERCEL_ARTIFACTS_OWNER.clone()),
            TestCase::new()
                .reason("just TURBO_TEAMID")
                .TURBO_TEAMID()
                .team_id(TURBO_TEAMID.clone()),
            //
            // Just get a team_slug
            // ------------------------------
            TestCase::new()
                .reason("just TURBO_TEAM")
                .TURBO_TEAM()
                .team_slug(TURBO_TEAM.clone()),
            //
            // just team_slug and team_id
            // ------------------------------
            TestCase::new()
                .reason("if we just have TURBO_TEAM+TURBO_TEAMID, that's ok")
                .TURBO_TEAM()
                .TURBO_TEAMID()
                .team_slug(TURBO_TEAM.clone())
                .team_id(TURBO_TEAMID.clone()),
            //
            // just set team_id and team_slug
            // ------------------------------
            TestCase::new()
                .reason("if we just have a TURBO_TEAM and TURBO_TEAMID we can use them both")
                .TURBO_TEAM()
                .TURBO_TEAMID()
                .team_id(TURBO_TEAMID.clone())
                .team_slug(TURBO_TEAM.clone()),
        ];

        for case in cases {
            let mut env: HashMap<OsString, OsString> = HashMap::new();

            if let Some(value) = &case.input.TURBO_TEAM {
                env.insert("turbo_team".into(), value.into());
            }
            if let Some(value) = &case.input.TURBO_TEAMID {
                env.insert("turbo_teamid".into(), value.into());
            }
            if let Some(value) = &case.input.TURBO_TOKEN {
                env.insert("turbo_token".into(), value.into());
            }
            if let Some(value) = &case.input.VERCEL_ARTIFACTS_OWNER {
                env.insert("vercel_artifacts_owner".into(), value.into());
            }
            if let Some(value) = &case.input.VERCEL_ARTIFACTS_TOKEN {
                env.insert("vercel_artifacts_token".into(), value.into());
            }

            let config = OverrideEnvVars::new(&env).unwrap();
            let reason = case.reason;

            assert_eq!(case.output.team_id, config.output.team_id, "{reason}");
            assert_eq!(case.output.team_slug, config.output.team_slug, "{reason}");
            assert_eq!(case.output.token, config.output.token, "{reason}");
        }
    }
}

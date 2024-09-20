use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub version: String,
    #[serde(rename = "$schema")]
    pub schema: Option<String>,
    // TODO: only one application can be marked as default
    pub applications: BTreeMap<String, Application>,
    pub options: Option<Options>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Options {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vercel: Option<VercelOptions>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VercelOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stitch_applications_in_preview: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bypass_deployment_protection_in_production: Option<bool>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Application {
    // default = true -> no routing
    // default = false -> requires routing
    pub default: bool,
    pub routing: Option<ZoneRouting>,
    pub development: Development,
    pub production: Host,
    pub metadata: Option<BTreeMap<String, String>>,
    pub federation: Option<Federation>,
    pub vercel: Option<Vercel>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Vercel {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment_protection_env_key: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Federation {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exposes: Vec<Module>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uses: Vec<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Module {
    pub name: String,
    pub path: String,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZoneRouting {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_prefix: Option<String>,
    pub matches: Vec<PathGroup>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathGroup {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<PathConfigurationOptions>,
    pub paths: Vec<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathConfigurationOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_to_default_application: Option<bool>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Development {
    pub local: Host,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<Host>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Host {
    pub protocol: Protocol,
    pub host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Http,
    Https,
}

#[cfg(test)]
mod test {
    use pretty_assertions::assert_eq;

    use super::*;

    // todo: make sure comments are allowed

    fn assert_round_trip<T>(input: &str) -> Result<(), serde_json::Error>
    where
        for<'a> T: Serialize + Deserialize<'a>,
    {
        let value: T = serde_json::from_str(input)?;
        let output = serde_json::to_string(&value)?;
        assert_eq!(
            input,
            output,
            "roundtrip failed for {:?}",
            std::any::type_name::<T>()
        );
        Ok(())
    }

    const EXAMPLE_HOST: &str = r#"{"protocol":"https","host":"example.com"}"#;
    const EXAMPLE_PATH_GROUP: &str =
        r#"{"group":"docs","options":{"routeToDefaultApplication":true},"paths":["docs/:path*"]}"#;

    #[test]
    fn test_round_trips() -> Result<(), serde_json::Error> {
        assert_round_trip::<Protocol>(r#""http""#)?;
        assert_round_trip::<Protocol>(r#""https""#)?;
        assert_round_trip::<Host>(EXAMPLE_HOST)?;
        assert_round_trip::<Host>(r#"{"protocol":"http","host":"example.com","port":3000}"#)?;
        assert_round_trip::<Development>(&format!("{{\"local\":{EXAMPLE_HOST}}}"))?;
        assert_round_trip::<Development>(&format!(
            "{{\"local\":{EXAMPLE_HOST},\"task\":\"dev\"}}"
        ))?;
        assert_round_trip::<Development>(&format!(
            "{{\"local\":{EXAMPLE_HOST},\"fallback\":{EXAMPLE_HOST}}}"
        ))?;
        assert_round_trip::<Vercel>(r#"{}"#)?;
        assert_round_trip::<Vercel>(r#"{"projectId":"foobar"}"#)?;
        assert_round_trip::<Vercel>(
            r#"{"projectId":"foobar","projectName":"secret","deploymentProtectionEnvKey":"MY_VAR"}"#,
        )?;
        assert_round_trip::<PathConfigurationOptions>(r#"{}"#)?;
        assert_round_trip::<PathConfigurationOptions>(r#"{"flag":"staging"}"#)?;
        assert_round_trip::<PathConfigurationOptions>(r#"{"routeToDefaultApplication":true}"#)?;
        assert_round_trip::<PathGroup>(r#"{"paths":["docs/:path*"]}"#)?;
        assert_round_trip::<PathGroup>(EXAMPLE_PATH_GROUP)?;
        assert_round_trip::<ZoneRouting>(&format!("{{\"matches\":[{EXAMPLE_PATH_GROUP}]}}"))?;
        assert_round_trip::<ZoneRouting>(&format!(
            "{{\"assetPrefix\":\"turbo\",\"matches\":[{EXAMPLE_PATH_GROUP}]}}"
        ))?;
        assert_round_trip::<Module>(r#"{"name":"lazy","path":"lazy.js"}"#)?;
        assert_round_trip::<Federation>(r#"{}"#)?;
        assert_round_trip::<Federation>(
            r#"{"exposes":[{"name":"lazy","path":"lazy.js"}],"uses":["lazy"]}"#,
        )?;
        assert_round_trip::<VercelOptions>(r#"{}"#)?;
        assert_round_trip::<VercelOptions>(
            r#"{"stitchApplicationsInPreview":true,"bypassDeploymentProtectionInProduction":false}"#,
        )?;
        assert_round_trip::<Options>(r#"{}"#)?;
        assert_round_trip::<Options>(r#"{"vercel":{}}"#)?;
        Ok(())
    }
}

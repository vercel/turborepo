use std::{collections::HashMap, default::Default, fmt::Write};

use anyhow::{bail, Result};
use crossterm::style::Stylize;
use serde::{Deserialize, Serialize};
use turbo_tasks::{
    primitives::{BoolVc, StringsVc},
    trace::TraceRawVcs,
    Value,
};
use turbo_tasks_fs::{FileSystemEntryType, FileSystemPathVc};
use turbopack::evaluate_context::node_evaluate_asset_context;
use turbopack_core::{
    asset::Asset,
    reference_type::{EntryReferenceSubType, ReferenceType},
    resolve::{
        find_context_file,
        options::{ImportMap, ImportMapping},
        FindContextFileResult,
    },
    source_asset::SourceAssetVc,
};
use turbopack_ecmascript::{
    chunk::EcmascriptChunkPlaceablesVc, EcmascriptInputTransformsVc, EcmascriptModuleAssetType,
    EcmascriptModuleAssetVc,
};
use turbopack_node::{
    evaluate::{evaluate, JavaScriptValue},
    execution_context::{ExecutionContext, ExecutionContextVc},
};

use crate::embed_js::next_asset;

#[turbo_tasks::value(serialization = "custom", eq = "manual")]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NextConfig {
    pub cross_origin: Option<String>,
    pub config_file: String,
    pub config_file_name: String,

    pub react_strict_mode: Option<bool>,
    pub experimental: ExperimentalConfig,
    pub images: ImageConfig,
    pub transpile_packages: Option<Vec<String>>,

    #[serde(flatten)]
    unsupported: UnsupportedNextConfig,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TraceRawVcs)]
#[serde(rename_all = "camelCase")]
struct AmpConfig {
    canonical_base: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TraceRawVcs)]
#[serde(rename_all = "camelCase")]
struct EslintConfig {
    dirs: Option<Vec<String>>,
    ignore_during_builds: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TraceRawVcs)]
#[serde(rename_all = "kebab-case")]
enum BuildActivityPositions {
    BottomRight,
    BottomLeft,
    TopRight,
    TopLeft,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TraceRawVcs)]
#[serde(rename_all = "camelCase")]
struct DevIndicatorsConfig {
    build_activity: bool,
    build_activity_position: BuildActivityPositions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TraceRawVcs)]
#[serde(rename_all = "camelCase")]
struct OnDemandEntriesConfig {
    max_inactive_age: f64,
    pages_buffer_length: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TraceRawVcs)]
#[serde(rename_all = "camelCase")]
struct HttpAgentConfig {
    keep_alive: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TraceRawVcs)]
#[serde(rename_all = "camelCase")]
struct DomainLocale {
    default_locale: String,
    domain: String,
    http: Option<bool>,
    locales: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TraceRawVcs)]
#[serde(rename_all = "camelCase")]
struct I18NConfig {
    default_locale: String,
    domains: Option<Vec<DomainLocale>>,
    locale_detection: Option<bool>,
    locales: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TraceRawVcs)]
#[serde(rename_all = "kebab-case")]
enum OutputType {
    Standalone,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TraceRawVcs)]
#[serde(rename_all = "camelCase")]
pub struct TypeScriptConfig {
    pub ignore_build_errors: Option<bool>,
    pub ts_config_path: Option<String>,
}

/// unsupported next config options
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TraceRawVcs)]
#[serde(rename_all = "camelCase")]
struct UnsupportedNextConfig {
    amp: AmpConfig,
    analytics_id: String,
    asset_prefix: String,
    base_path: String,
    clean_dist_dir: bool,
    compiler: Option<CompilerConfig>,
    compress: bool,
    dev_indicators: DevIndicatorsConfig,
    dist_dir: String,
    env: HashMap<String, String>,
    eslint: EslintConfig,
    exclude_default_moment_locales: bool,
    // this can be a function in js land?
    export_path_map: Option<serde_json::Value>,
    // this is a function in js land?
    generate_build_id: Option<serde_json::Value>,
    generate_etags: bool,
    // this is a function in js land?
    headers: Option<serde_json::Value>,
    http_agent_options: HttpAgentConfig,
    i18n: Option<I18NConfig>,
    on_demand_entries: OnDemandEntriesConfig,
    optimize_fonts: bool,
    output: Option<OutputType>,
    output_file_tracing: bool,
    page_extensions: Vec<String>,
    powered_by_header: bool,
    production_browser_source_maps: bool,
    public_runtime_config: HashMap<String, serde_json::Value>,
    react_strict_mode: Option<bool>,
    // this is a function in js land?
    redirects: Option<serde_json::Value>,
    // this is a function in js land?
    rewrites: Option<serde_json::Value>,
    sass_options: HashMap<String, serde_json::Value>,
    server_runtime_config: HashMap<String, serde_json::Value>,
    static_page_generation_timeout: f64,
    swc_minify: bool,
    target: String,
    trailing_slash: bool,
    typescript: TypeScriptConfig,
    use_file_system_public_routes: bool,
    webpack: Option<serde_json::Value>,
}

impl Default for UnsupportedNextConfig {
    fn default() -> Self {
        UnsupportedNextConfig {
            amp: AmpConfig {
                canonical_base: Some("".to_string()),
            },
            analytics_id: "".to_string(),
            asset_prefix: "".to_string(),
            base_path: "".to_string(),
            clean_dist_dir: true,
            compiler: None,
            compress: true,
            dev_indicators: DevIndicatorsConfig {
                build_activity: true,
                build_activity_position: BuildActivityPositions::BottomRight,
            },
            dist_dir: ".next".to_string(),
            env: Default::default(),
            eslint: EslintConfig {
                dirs: None,
                ignore_during_builds: Some(false),
            },
            exclude_default_moment_locales: true,
            export_path_map: None,
            generate_build_id: None,
            generate_etags: true,
            headers: None,
            http_agent_options: HttpAgentConfig { keep_alive: true },
            i18n: None,
            on_demand_entries: OnDemandEntriesConfig {
                max_inactive_age: 15.0 * 1000.0,
                pages_buffer_length: 2.0,
            },
            optimize_fonts: true,
            output: None,
            output_file_tracing: true,
            page_extensions: vec![
                "tsx".to_string(),
                "ts".to_string(),
                "jsx".to_string(),
                "js".to_string(),
            ],
            powered_by_header: true,
            production_browser_source_maps: false,
            public_runtime_config: Default::default(),
            react_strict_mode: None,
            redirects: None,
            rewrites: None,
            sass_options: Default::default(),
            server_runtime_config: Default::default(),
            static_page_generation_timeout: 60.0,
            swc_minify: true,
            target: "server".to_string(),
            trailing_slash: false,
            typescript: TypeScriptConfig {
                ignore_build_errors: Some(false),
                ts_config_path: None,
            },
            use_file_system_public_routes: true,
            webpack: None,
        }
    }
}

#[turbo_tasks::value(eq = "manual")]
#[derive(Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImageConfig {
    pub device_sizes: Vec<u16>,
    pub image_sizes: Vec<u16>,
    pub path: String,
    pub loader: ImageLoader,
    pub domains: Vec<String>,
    pub disable_static_images: bool,
    #[serde(rename(deserialize = "minimumCacheTTL"))]
    pub minimum_cache_ttl: u32,
    pub formats: Vec<ImageFormat>,
    #[serde(rename(deserialize = "dangerouslyAllowSVG"))]
    pub dangerously_allow_svg: bool,
    pub content_security_policy: String,
    pub remote_patterns: Vec<RemotePattern>,
    pub unoptimized: bool,
}

impl Default for ImageConfig {
    fn default() -> Self {
        // https://github.com/vercel/next.js/blob/327634eb/packages/next/shared/lib/image-config.ts#L100-L114
        Self {
            device_sizes: vec![640, 750, 828, 1080, 1200, 1920, 2048, 3840],
            image_sizes: vec![16, 32, 48, 64, 96, 128, 256, 384],
            path: "/_next/image".to_string(),
            loader: ImageLoader::Default,
            domains: vec![],
            disable_static_images: false,
            minimum_cache_ttl: 60,
            formats: vec![ImageFormat::Webp],
            dangerously_allow_svg: false,
            content_security_policy: "".to_string(),
            remote_patterns: vec![],
            unoptimized: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TraceRawVcs)]
#[serde(rename_all = "kebab-case")]
pub enum ImageLoader {
    Default,
    Imgix,
    Cloudinary,
    Akamai,
    Custom,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TraceRawVcs)]
pub enum ImageFormat {
    #[serde(rename = "image/webp")]
    Webp,
    #[serde(rename = "image/avif")]
    Avif,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TraceRawVcs)]
#[serde(rename_all = "camelCase")]
pub struct RemotePattern {
    pub protocol: Option<RemotePatternProtocal>,
    pub hostname: String,
    pub port: Option<String>,
    pub pathname: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TraceRawVcs)]
#[serde(rename_all = "kebab-case")]
pub enum RemotePatternProtocal {
    Http,
    Https,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TraceRawVcs)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentalConfig {
    pub server_components_external_packages: Option<Vec<String>>,
    pub app_dir: Option<bool>,

    // we only have swc
    force_swc_transforms: Option<bool>,

    #[serde(flatten)]
    unsupported: UnsupportedExperimentalConfig,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TraceRawVcs)]
#[serde(rename_all = "kebab-case")]
enum MiddlewarePrefetchType {
    Strict,
    Flexible,
}

/// unsupported experimental config options
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TraceRawVcs)]
#[serde(rename_all = "camelCase")]
struct UnsupportedExperimentalConfig {
    adjust_font_fallbacks: Option<bool>,
    adjust_font_fallbacks_with_size_adjust: Option<bool>,
    allow_middleware_response_body: Option<bool>,
    amp: Option<serde_json::Value>,
    cpus: Option<f64>,
    cra_compat: Option<bool>,
    disable_optimized_loading: Option<bool>,
    disable_postcss_preset_env: Option<bool>,
    enable_undici: Option<bool>,
    esm_externals: Option<serde_json::Value>,
    external_dir: Option<bool>,
    fallback_node_polyfills: Option<bool>,
    fetch_cache: Option<bool>,
    font_loaders: Option<serde_json::Value>,
    fully_specified: Option<bool>,
    gzip_size: Option<bool>,
    incremental_cache_handler_path: Option<String>,
    isr_flush_to_disk: Option<bool>,
    isr_memory_cache_size: Option<f64>,
    large_page_data_bytes: Option<f64>,
    legacy_browsers: Option<bool>,
    manual_client_base_path: Option<bool>,
    mdx_rs: Option<serde_json::Value>,
    middleware_prefetch: Option<MiddlewarePrefetchType>,
    modularize_imports: Option<serde_json::Value>,
    new_next_link_behavior: Option<bool>,
    next_script_workers: Option<bool>,
    optimistic_client_cache: Option<bool>,
    optimize_css: Option<serde_json::Value>,
    output_file_tracing_ignores: Option<Vec<String>>,
    output_file_tracing_root: Option<String>,
    page_env: Option<bool>,
    profiling: Option<bool>,
    proxy_timeout: Option<f64>,
    runtime: Option<serde_json::Value>,
    scroll_restoration: Option<bool>,
    shared_pool: Option<bool>,
    skip_middleware_url_normalize: Option<bool>,
    skip_trailing_slash_redirect: Option<bool>,
    sri: Option<serde_json::Value>,
    swc_file_reading: Option<bool>,
    swc_minify: Option<bool>,
    swc_minify_debug_options: Option<serde_json::Value>,
    swc_plugins: Option<serde_json::Value>,
    swc_trace_profiling: Option<bool>,
    transpile_packages: Option<Vec<String>>,
    turbotrace: Option<serde_json::Value>,
    url_imports: Option<serde_json::Value>,
    web_vitals_attribution: Option<serde_json::Value>,
    worker_threads: Option<bool>,
}

impl Default for UnsupportedExperimentalConfig {
    fn default() -> Self {
        UnsupportedExperimentalConfig {
            adjust_font_fallbacks: Some(false),
            adjust_font_fallbacks_with_size_adjust: Some(false),
            allow_middleware_response_body: None,
            amp: None,
            cpus: Some(1.max(num_cpus::get() - 1) as f64),
            cra_compat: Some(false),
            disable_optimized_loading: Some(false),
            disable_postcss_preset_env: None,
            enable_undici: Some(false),
            esm_externals: Some(serde_json::Value::Bool(true)),
            external_dir: Some(false),
            fallback_node_polyfills: None,
            fetch_cache: Some(false),
            font_loaders: None,
            fully_specified: Some(false),
            gzip_size: Some(true),
            incremental_cache_handler_path: None,
            isr_flush_to_disk: Some(true),
            // default to 50MB limit
            isr_memory_cache_size: Some(50.0 * 1024.0 * 1024.0),
            // default to 128KB
            large_page_data_bytes: Some(128.0 * 1000.0),
            legacy_browsers: Some(false),
            manual_client_base_path: Some(false),
            mdx_rs: None,
            middleware_prefetch: Some(MiddlewarePrefetchType::Flexible),
            modularize_imports: None,
            new_next_link_behavior: Some(true),
            next_script_workers: Some(false),
            optimistic_client_cache: Some(true),
            optimize_css: Some(serde_json::Value::Bool(false)),
            output_file_tracing_ignores: None,
            output_file_tracing_root: Some("".to_string()),
            page_env: Some(false),
            profiling: Some(false),
            proxy_timeout: None,
            runtime: None,
            scroll_restoration: Some(false),
            shared_pool: Some(true),
            skip_middleware_url_normalize: None,
            skip_trailing_slash_redirect: None,
            sri: None,
            swc_file_reading: Some(true),
            swc_minify: None,
            swc_minify_debug_options: None,
            swc_plugins: None,
            swc_trace_profiling: Some(false),
            turbotrace: None,
            transpile_packages: None,
            url_imports: None,
            web_vitals_attribution: None,
            worker_threads: Some(false),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TraceRawVcs)]
#[serde(rename_all = "camelCase")]
pub struct CompilerConfig {
    pub react_remove_properties: Option<bool>,
    pub relay: Option<RelayConfig>,
    pub remove_console: Option<RemoveConsoleConfig>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TraceRawVcs)]
#[serde(untagged, rename_all = "camelCase")]
pub enum ReactRemoveProperties {
    Boolean(bool),
    Config { properties: Option<Vec<String>> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TraceRawVcs)]
#[serde(rename_all = "camelCase")]
pub struct RelayConfig {
    pub src: String,
    pub artifact_directory: Option<String>,
    pub language: Option<RelayLanguage>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TraceRawVcs)]
#[serde(untagged, rename_all = "lowercase")]
pub enum RelayLanguage {
    TypeScript,
    Flow,
    JavaScript,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TraceRawVcs)]
#[serde(untagged)]
pub enum RemoveConsoleConfig {
    Boolean(bool),
    Config { exclude: Option<Vec<String>> },
}

#[turbo_tasks::value_impl]
impl NextConfigVc {
    #[turbo_tasks::function]
    pub async fn server_component_externals(self) -> Result<StringsVc> {
        Ok(StringsVc::cell(
            self.await?
                .experimental
                .server_components_external_packages
                .as_ref()
                .cloned()
                .unwrap_or_default(),
        ))
    }

    #[turbo_tasks::function]
    pub async fn app_dir(self) -> Result<BoolVc> {
        Ok(BoolVc::cell(
            self.await?
                .experimental
                .app_dir
                .as_ref()
                .cloned()
                .unwrap_or_default(),
        ))
    }

    #[turbo_tasks::function]
    pub async fn image_config(self) -> Result<ImageConfigVc> {
        Ok(self.await?.images.clone().cell())
    }

    #[turbo_tasks::function]
    pub async fn transpile_packages(self) -> Result<StringsVc> {
        Ok(StringsVc::cell(
            self.await?.transpile_packages.clone().unwrap_or_default(),
        ))
    }
}

fn next_configs() -> StringsVc {
    StringsVc::cell(
        ["next.config.mjs", "next.config.js"]
            .into_iter()
            .map(ToOwned::to_owned)
            .collect(),
    )
}

#[turbo_tasks::function]
pub async fn load_next_config(execution_context: ExecutionContextVc) -> Result<NextConfigVc> {
    let ExecutionContext {
        project_root,
        intermediate_output_path,
    } = *execution_context.await?;
    let mut import_map = ImportMap::default();

    import_map.insert_exact_alias("next", ImportMapping::External(None).into());
    import_map.insert_wildcard_alias("next/", ImportMapping::External(None).into());

    let context = node_evaluate_asset_context(Some(import_map.cell()));
    let find_config_result = find_context_file(project_root, next_configs());
    let config_asset = match &*find_config_result.await? {
        FindContextFileResult::Found(config_path, _) => Some(SourceAssetVc::new(*config_path)),
        FindContextFileResult::NotFound(_) => None,
    };

    let runtime_entries = config_asset.map(|config_asset| {
        // TODO this is a hack to add the config to the bundling to make it watched
        let config_chunk = EcmascriptModuleAssetVc::new(
            config_asset.into(),
            context,
            Value::new(EcmascriptModuleAssetType::Ecmascript),
            EcmascriptInputTransformsVc::cell(vec![]),
            context.environment(),
        )
        .as_ecmascript_chunk_placeable();
        EcmascriptChunkPlaceablesVc::cell(vec![config_chunk])
    });
    let asset_path = config_asset
        .map_or(project_root, |a| a.path())
        .join("load-next-config.js");
    let load_next_config_asset = context.process(
        next_asset(asset_path, "entry/config/next.js"),
        Value::new(ReferenceType::Entry(EntryReferenceSubType::Undefined)),
    );
    let config_value = evaluate(
        project_root,
        load_next_config_asset,
        project_root,
        config_asset.map_or(project_root, |c| c.path()),
        context,
        intermediate_output_path,
        runtime_entries,
        vec![],
    )
    .await?;
    match &*config_value {
        JavaScriptValue::Value(val) => {
            let next_config: NextConfig = serde_json::from_reader(val.read())?;
            let next_config = next_config.cell();

            validate_next_config(project_root, next_config).await?;

            Ok(next_config)
        }
        JavaScriptValue::Error => Ok(NextConfig::default().cell()),
        JavaScriptValue::Stream(_) => {
            unimplemented!("Stream not supported now");
        }
    }
}

const BABEL_CONFIG_FILES: &[&'static str] = &[
    ".babelrc",
    ".babelrc.json",
    ".babelrc.js",
    ".babelrc.mjs",
    ".babelrc.cjs",
    "babel.config.js",
    "babel.config.json",
    "babel.config.mjs",
    "babel.config.cjs",
];

async fn get_babel_config_file(project_path: FileSystemPathVc) -> Result<Option<FileSystemPathVc>> {
    for filename in BABEL_CONFIG_FILES {
        let file_path = project_path.join(filename);
        let file_type = file_path.get_type().await?;

        if *file_type == FileSystemEntryType::File {
            return Ok(Some(file_path));
        }
    }

    Ok(None)
}

fn compare_to_default<T: Default + Serialize + PartialEq>(unsupported: &T) -> Result<Vec<String>> {
    let default_cfg = T::default();

    if *unsupported == default_cfg {
        return Ok(vec![]);
    }

    let value = serde_json::to_value(unsupported)?;
    let default_value = serde_json::to_value(&default_cfg)?;

    let serde_json::Value::Object(map) = value else {
        bail!("can only compare maps");
    };
    let serde_json::Value::Object(default_map) = default_value else {
        bail!("can only compare maps");
    };

    let mut defined_options = vec![];
    for (k, v) in map.iter() {
        if default_map[k] != *v {
            defined_options.push(k.clone());
        }
    }

    Ok(defined_options)
}

const SUPPORTED_OPTIONS: &[&'static str] = &[
    "images",
    "reactStrictMode",
    "configFileName",
    "swcMinify",
    "experimental.appDir",
    "experimental.serverComponentsExternalPackages",
];

async fn validate_next_config(
    project_path: FileSystemPathVc,
    next_config: NextConfigVc,
) -> Result<()> {
    let babelrc = get_babel_config_file(project_path).await?;

    let thank_you_msg = "\nThank you for trying Next.js v13 with Turbopack! As a \
                         reminder,\nTurbopack is currently in alpha and not yet ready for \
                         production.\nWe appreciate your ongoing support as we work to make it \
                         ready\nfor everyone.\n";

    let mut unsupported_parts = String::new();

    let next_config = &*next_config.await?;

    let mut unsupported_options = compare_to_default(&next_config.unsupported)?;
    let unsupported_experimental_options =
        compare_to_default(&next_config.experimental.unsupported)?;

    for unsupported_option in unsupported_experimental_options {
        unsupported_options.push(format!("experimental.{}", unsupported_option));
    }

    let has_non_default_config = unsupported_options.len() != 0;

    let has_warning_or_error = babelrc.is_some() || has_non_default_config;
    if has_warning_or_error {
        println!("{}", thank_you_msg);
    } else {
        println!("{}", thank_you_msg.dim());
    }

    let mut feedback_message = format!(
        "Learn more about Next.js v13 and Turbopack: {}\nPlease direct feedback to: {}\n",
        "https://nextjs.link/with-turbopack".underlined(),
        "https://nextjs.link/turbopack-feedback".underlined()
    )
    .stylize();

    if !has_warning_or_error {
        feedback_message = feedback_message.dim()
    }

    if let Some(babelrc) = babelrc {
        write!(
            unsupported_parts,
            "\n- Babel detected ({})\n  {}",
            babelrc.await?.file_name().cyan(),
            "Babel is not yet supported. To use Turbopack at the moment,\n  you'll need to remove \
             your usage of Babel."
                .dim()
        )?;
    }

    if has_non_default_config {
        writeln!(
            unsupported_parts,
            "\n- Unsupported Next.js configuration option(s) ({}):",
            "next.config.js".cyan(),
        )?;

        for option in unsupported_options {
            writeln!(unsupported_parts, "    - {}", option.red())?;
        }

        writeln!(
            unsupported_parts,
            "{}",
            "  To use Turbopack, remove those configuration options.\n  The only supported \
             options are:"
                .dim()
        )?;

        for option in SUPPORTED_OPTIONS {
            writeln!(unsupported_parts, "    - {}", option.cyan().dim())?;
        }
    }

    if !unsupported_parts.is_empty() {
        // const pkgManager = getPkgManager(dir)
        let pkg_manager = "npm";

        let commands = format!(
            "{}\n  cd with-turbopack-app\n  {pkg_manager} run dev\n",
            format!(
                "{} --example with-turbopack with-turbopack-app",
                if pkg_manager == "npm" {
                    "npx create-next-app".to_string()
                } else {
                    format!("{pkg_manager} create next-app")
                }
            )
            .bold()
            .cyan(),
        );

        println!(
            "{} You are using configuration and/or tools that are not yet\nsupported by Next.js \
             v13 with Turbopack:\n{unsupported_parts}\nIf you cannot make the changes above, but \
             still want to try out\nNext.js v13 with Turbopack, create the Next.js v13 playground \
             app\nby running the following commands:\n\n  {}",
            "Error:".bold().red(),
            commands,
        );
        println!("{}", feedback_message);

        std::process::exit(1);
    }

    println!("{}", feedback_message);

    Ok(())
}

pub use turborepo_prune::{DEFAULT_OUTPUT_DIR, Error};
use turborepo_telemetry::events::command::CommandEventBuilder;

use super::CommandBase;

pub async fn prune(
    base: &CommandBase,
    scope: &[String],
    docker: bool,
    production: bool,
    output_dir: &str,
    use_gitignore: bool,
    telemetry: CommandEventBuilder,
) -> Result<(), Error> {
    let input = turborepo_prune::PruneInput {
        repo_root: base.repo_root.clone(),
        color_config: base.color_config,
        scope: scope.to_vec(),
        docker,
        production,
        output_dir: output_dir.to_owned(),
        use_gitignore,
        allow_missing_package_manager: base.opts().repo_opts.allow_no_package_manager,
        future_flags: base.opts().future_flags,
        root_turbo_json_path: base.opts().repo_opts.root_turbo_json_path.clone(),
    };

    turborepo_prune::prune(input, telemetry).await
}

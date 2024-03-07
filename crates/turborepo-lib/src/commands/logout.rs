use turborepo_auth::{logout as auth_logout, LogoutOptions};
use turborepo_telemetry::events::command::CommandEventBuilder;

use crate::{cli::Error, commands::CommandBase};

pub async fn logout(
    base: &mut CommandBase,
    invalidate: bool,
    _telemetry: CommandEventBuilder,
) -> Result<(), Error> {
    auth_logout(&LogoutOptions {
        ui: &base.ui,
        api_client: &base.api_client()?,
        path: &base.global_config_path()?,
        invalidate,
    })
    .await
    .map_err(Error::from)
}

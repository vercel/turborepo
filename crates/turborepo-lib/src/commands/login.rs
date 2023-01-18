use log::{debug, info, warn};

use crate::{config::RepoConfig, get_version};

const DEFAULT_HOST_NAME: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 9789;

pub fn login(repo_config: RepoConfig) {
    let login_url_base = &repo_config.login_url;
    debug!("turbo v{}", get_version());
    debug!("api url: {}", repo_config.api_url);
    debug!("login url: {login_url_base}");

    let redirect_url = format!("http://{DEFAULT_HOST_NAME}:{DEFAULT_PORT}");
    let login_url = format!("{login_url_base}/turborepo/token?redirect_uri={redirect_url}");

    info!(">>> Opening browser to {login_url}");
    direct_user_to_url(&login_url);
}

fn direct_user_to_url(url: &str) {
    if webbrowser::open(url).is_err() {
        warn!("Failed to open browser. Please visit {url} in your browser.");
    }
}

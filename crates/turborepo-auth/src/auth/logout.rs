use anyhow::Result;
use tracing::error;
use turborepo_ui::{GREY, UI};

pub fn logout<F>(ui: &UI, mut set_token: F) -> Result<()>
where
    F: FnMut() -> Result<()>,
{
    if let Err(err) = set_token() {
        error!("could not logout. Something went wrong: {}", err);
        return Err(err);
    }

    println!("{}", ui.apply(GREY.apply_to(">>> Logged out")));
    Ok(())
}

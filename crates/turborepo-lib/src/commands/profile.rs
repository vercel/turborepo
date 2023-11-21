use turborepo_profiles;

use crate::{cli::Error, CommandBase};

pub async fn profile(base: &CommandBase) -> Result<(), Error> {
    let profiles = turborepo_profiles::Profiles::read_from_file(&base.profile_config_path());
    println!("{:#?}", profiles);
    Ok(())
}

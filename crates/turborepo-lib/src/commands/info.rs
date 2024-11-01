use std::{env, io};

use thiserror::Error;

use super::CommandBase;
use crate::get_version;

#[derive(Debug, Error)]
pub enum Error {
    #[error("could not get path to turbo binary: {0}")]
    NoCurrentExe(#[from] io::Error),
}

pub fn run(base: CommandBase) -> Result<(), Error> {
    println!("CLI:");
    println!("   Version: {}", base.version);
    println!(
        "   Location: {}",
        std::env::current_exe()?.to_string_lossy()
    );
    println!("");

    println!("Platform:");
    println!("   Architecture: {}", std::env::consts::ARCH);
    println!("   Operating System: {}", std::env::consts::OS);
    println!("");

    println!("Environment:");
    println!("   CI: {:#?}", turborepo_ci::Vendor::get_name());
    println!(
        "   Terminal (TERM): {}",
        env::var("TERM").unwrap_or_else(|_| "unknown".to_owned())
    );

    println!(
        "   Terminal program (TERM_PROGRAM): {}",
        env::var("TERM_PROGRAM").unwrap_or_else(|_| "unknown".to_owned())
    );
    println!(
        "   Terminal program version (TERM_PROGRAM_VERSION): {}",
        env::var("TERM_PROGRAM_VERSION").unwrap_or_else(|_| "unknown".to_owned())
    );
    println!(
        "   Shell (SHELL): {}",
        env::var("SHELL").unwrap_or_else(|_| "unknown".to_owned())
    );
    println!("   stdin: {}", turborepo_ci::is_ci());
    println!("");

    println!("Turborepo System Environment Variables:");
    for (key, value) in env::vars().fil {
        if key.starts_with("TURBO_") {
            println!("   {}: {}", key, value);
        }
    }

    Ok(())
}

use std::{
    path::Path,
    process::{Child, Command, Stdio},
};

use anyhow::{anyhow, Context, Result};
use regex::Regex;

use crate::{
    bundlers::Bundler,
    util::{
        npm::{self, NpmPackage},
        wait_for_match,
    },
};

pub struct RsPack;
impl Bundler for RsPack {
    fn get_name(&self) -> &str {
        "rspack CSR"
    }

    fn prepare(&self, install_dir: &Path) -> Result<()> {
        npm::install(
            install_dir,
            &[
                NpmPackage::new("@rspack/cli", "*"),
                NpmPackage::new("react-refresh", "^0.14.0"),
            ],
        )
        .context("failed to install from npm")?;

        Ok(())
    }

    fn start_server(&self, test_dir: &Path) -> Result<(Child, String)> {
        let mut proc = Command::new("node")
            .args([
                (test_dir
                    .join("node_modules")
                    .join("@rspack")
                    .join("cli")
                    .join("bin")
                    .join("rspack")
                    .to_str()
                    .unwrap()),
                "--port",
                "0",
            ])
            .env("NO_COLOR", "1")
            .current_dir(test_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("failed to run `rspack dev` command")?;

        let addr = wait_for_match(
            proc.stderr
                .as_mut()
                .ok_or_else(|| anyhow!("missing stderr"))?,
            Regex::new("\\[rspack\\-dev\\-server\\] Loopback:\\s+(.*)")?,
        )
        .ok_or_else(|| anyhow!("failed to find devserver address"))?;

        Ok((proc, addr))
    }
}

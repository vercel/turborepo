use std::{
    fs,
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

pub struct Vite {
    swc: bool,
}

impl Vite {
    pub fn new(swc: bool) -> Self {
        Vite { swc }
    }
}

impl Bundler for Vite {
    fn get_name(&self) -> &str {
        if self.swc {
            "Vite SWC CSR"
        } else {
            "Vite CSR"
        }
    }

    fn prepare(&self, install_dir: &Path) -> Result<()> {
        let mut packages = vec![NpmPackage::new("vite", "3.0.9")];
        if self.swc {
            packages.push(NpmPackage::new("vite-plugin-swc-react-refresh", "2.2.0"));
        } else {
            packages.push(NpmPackage::new("@vitejs/plugin-react", "2.1.0"));
        };
        npm::install(install_dir, &packages).context("failed to install from npm")?;

        fs::write(
            install_dir.join("vite.config.js"),
            if self.swc {
                include_bytes!("vite.swc.config.js") as &[u8]
            } else {
                include_bytes!("vite.config.js") as &[u8]
            },
        )?;

        Ok(())
    }

    fn start_server(&self, test_dir: &Path) -> Result<(Child, String)> {
        let mut proc = Command::new("node")
            .args([
                (test_dir
                    .join("node_modules")
                    .join("vite")
                    .join("bin")
                    .join("vite.js")
                    .to_str()
                    .unwrap()),
                "--port",
                "0",
            ])
            .env("NO_COLOR", "1")
            .current_dir(test_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("failed to run `vite` command")?;

        let addr = wait_for_match(
            proc.stdout
                .as_mut()
                .ok_or_else(|| anyhow!("missing stdout"))?,
            Regex::new("Local:\\s+(.*)")?,
        )
        .ok_or_else(|| anyhow!("failed to find devserver address"))?;

        Ok((proc, addr))
    }
}

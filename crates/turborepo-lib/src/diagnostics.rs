//! Diagnostic infrastructure for Turborepo.
//!
//! Most of the diagnostics implementation has been moved to the `turborepo-diagnostics`
//! crate. This module re-exports the public API and contains diagnostics that depend
//! on turborepo-lib internals.

use std::sync::Arc;

use tokio::sync::Mutex;

// Re-export everything from the new crate
pub use turborepo_diagnostics::{
    DaemonDiagnostic, Diagnostic, DiagnosticChannel, DiagnosticMessage, GitDaemonDiagnostic,
    LSPDiagnostic, UpdateDiagnostic,
};

use crate::commands::{
    link::{self, link},
    CommandBase,
};

/// a struct that checks and prompts the user to enable remote cache
pub struct RemoteCacheDiagnostic(pub Arc<Mutex<CommandBase>>);

impl RemoteCacheDiagnostic {
    pub fn new(base: CommandBase) -> Self {
        Self(Arc::new(Mutex::new(base)))
    }
}

impl Diagnostic for RemoteCacheDiagnostic {
    fn name(&self) -> &'static str {
        "vercel.auth"
    }

    fn execute(&self, chan: DiagnosticChannel) {
        let base = self.0.clone();
        tokio::task::spawn(async move {
            chan.started("Remote Cache".to_string()).await;

            let (has_team_id, has_team_slug) = {
                let base = base.lock().await;
                (
                    base.opts().api_client_opts.team_id.is_some(),
                    base.opts().api_client_opts.team_slug.is_some(),
                )
            };

            chan.log_line("Checking credentials".to_string()).await;

            if has_team_id || has_team_slug {
                chan.done("Remote Cache enabled".to_string()).await;
                return;
            }

            let result = {
                chan.log_line("Linking to remote cache".to_string()).await;
                let mut base = base.lock().await;
                let Some((stopped, resume)) = chan.suspend().await else {
                    // the sender (terminal) was shut, ignore
                    return;
                };
                stopped.await.unwrap();
                let link_res = link(&mut base, None, false, false).await;
                resume.send(()).unwrap();
                link_res
            };

            match result {
                Ok(_) => {
                    chan.log_line("Linked".to_string()).await;
                    chan.done("Remote Cache enabled".to_string()).await
                }
                Err(link::Error::NotLinking) => {
                    chan.not_applicable("Remote Cache opted out".to_string())
                        .await
                }
                Err(e) => {
                    chan.failed(format!("Failed to link: {e}")).await;
                }
            }
        });
    }
}

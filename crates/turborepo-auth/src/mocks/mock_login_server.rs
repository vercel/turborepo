use std::sync::{atomic::AtomicUsize, Arc};

use async_trait::async_trait;
use tokio::sync::OnceCell;

use crate::{login_server::LoginServer, Error};

pub struct MockLoginServer {
    pub hits: Arc<AtomicUsize>,
}

#[async_trait]
impl LoginServer for MockLoginServer {
    async fn run(
        &self,
        _: u16,
        _: String,
        login_token: Arc<OnceCell<String>>,
    ) -> Result<(), Error> {
        self.hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        login_token
            .set(turborepo_vercel_api_mock::EXPECTED_TOKEN.to_string())
            .unwrap();
        Ok(())
    }
    fn open_web_browser(&self, _: &str) -> std::io::Result<()> {
        Ok(())
    }
}

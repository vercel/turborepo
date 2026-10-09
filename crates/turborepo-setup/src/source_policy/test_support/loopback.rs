//! Literal-loopback, owned-lifetime HTTP fixture. No proxies, environment,
//! production endpoint override, or background process survives the fixture.
#![allow(clippy::unwrap_used)] // Failures in this explicit fixture seam fail the test.
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

pub struct LoopbackServer {
    origin: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl LoopbackServer {
    /// Standard Node + pnpm 10 fixture routes retaining official provenance.
    pub fn node_pnpm(
        node_path: String,
        node: Vec<u8>,
        pnpm: Vec<u8>,
        corrupt: bool,
        before_response: impl Fn(&str) + Send + 'static,
    ) -> std::io::Result<Self> {
        use base64::{Engine, engine::general_purpose::STANDARD};
        use sha2::{Digest, Sha512};
        let metadata = serde_json::json!({"name":"pnpm","version":"10.0.0","dist":{
            "tarball":"https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz",
            "integrity":format!("sha512-{}", STANDARD.encode(Sha512::digest(&pnpm)))
        }})
        .to_string()
        .into_bytes();
        Self::new(
            [
                (node_path, node),
                ("/pnpm/10.0.0".into(), metadata),
                (
                    "/pnpm/-/pnpm-10.0.0.tgz".into(),
                    if corrupt { b"corrupt".to_vec() } else { pnpm },
                ),
            ],
            before_response,
        )
    }

    pub fn new(
        routes: impl IntoIterator<Item = (String, Vec<u8>)>,
        before_response: impl Fn(&str) + Send + 'static,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let origin = format!("http://{}", listener.local_addr()?);
        let routes: BTreeMap<_, _> = routes.into_iter().collect();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = requests.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let worker = thread::spawn(move || {
            for socket in listener.incoming() {
                if stopping.load(Ordering::SeqCst) {
                    break;
                }
                let mut socket = socket.unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = String::new();
                BufReader::new(&socket)
                    .take(8192)
                    .read_line(&mut request)
                    .unwrap();
                let path = request.split_whitespace().nth(1).unwrap();
                log.lock().unwrap().push(path.to_owned());
                before_response(path);
                let (status, body) = routes
                    .get(path)
                    .map_or(("404 Not Found", &b"unexpected fixture request"[..]), |b| {
                        ("200 OK", b.as_slice())
                    });
                write!(
                    socket,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                socket.write_all(body).unwrap();
            }
        });
        Ok(Self {
            origin,
            requests,
            stop,
            worker: Some(worker),
        })
    }
    pub fn origin(&self) -> &str {
        &self.origin
    }
    pub fn hits(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}
impl Drop for LoopbackServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(&self.origin[7..]);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

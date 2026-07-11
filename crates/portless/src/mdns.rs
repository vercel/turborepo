//! LAN address detection, change monitoring, and native mDNS publication.

use std::{
    collections::HashMap,
    error::Error,
    fmt,
    future::Future,
    io,
    net::{IpAddr, Ipv4Addr},
    pin::Pin,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    thread,
    time::Duration,
};

use tokio::{net::UdpSocket, task::JoinHandle};

const PROBE_HOST: &str = "1.1.1.1:53";
const NO_ROUTE_IP: Ipv4Addr = Ipv4Addr::UNSPECIFIED;
pub const LAN_IP_POLL_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkInterfaceRow {
    pub name: String,
    pub address: Ipv4Addr,
    pub mac: String,
    pub internal: bool,
}

pub async fn probe_default_route_ipv4() -> io::Result<Ipv4Addr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await?;
    socket.connect(PROBE_HOST).await?;
    match socket.local_addr()?.ip() {
        IpAddr::V4(ip) if ip != NO_ROUTE_IP => Ok(ip),
        _ => Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "No route to host",
        )),
    }
}

/// Return the default-route IPv4 only when its interface is a physical,
/// non-loopback interface according to Portless/`lan-network` filtering.
pub fn filter_local_network_ip(
    probed_ip: Ipv4Addr,
    interfaces: &[NetworkInterfaceRow],
) -> Option<Ipv4Addr> {
    if probed_ip == NO_ROUTE_IP || probed_ip.is_loopback() {
        return None;
    }
    interfaces
        .iter()
        .find(|row| row.address == probed_ip)
        .filter(|row| !row.address.is_loopback() && !is_internal_interface(row))
        .map(|row| row.address)
}

pub fn is_internal_interface(row: &NetworkInterfaceRow) -> bool {
    if row.internal {
        return true;
    }
    let mac = parse_mac(&row.mac);
    if !mac.is_empty() && mac.iter().all(|byte| *byte == 0) {
        return true;
    }
    if mac.starts_with(&[0x00, 0x15, 0x5d]) {
        return true;
    }
    if row.name.contains("vEthernet") {
        return true;
    }
    row.name.strip_prefix("bridge").is_some_and(|suffix| {
        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn parse_mac(value: &str) -> Vec<u8> {
    value
        .split(':')
        .take(16)
        .map(|part| u8::from_str_radix(part, 16).unwrap_or_default())
        .collect()
}

pub trait NetworkInterfaceProvider: Send + Sync {
    fn interfaces(&self) -> io::Result<Vec<NetworkInterfaceRow>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemNetworkInterfaceProvider;

impl NetworkInterfaceProvider for SystemNetworkInterfaceProvider {
    fn interfaces(&self) -> io::Result<Vec<NetworkInterfaceRow>> {
        #[cfg(target_os = "linux")]
        {
            linux_interfaces()
        }
        #[cfg(target_os = "macos")]
        {
            macos_interfaces()
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "LAN interface enumeration is unsupported on this platform",
            ))
        }
    }
}

#[cfg(target_os = "linux")]
fn linux_interfaces() -> io::Result<Vec<NetworkInterfaceRow>> {
    use std::{
        ffi::{c_char, c_int, c_uint, c_void, CStr},
        ptr,
    };

    const AF_INET: u16 = 2;
    const IFF_LOOPBACK: c_uint = 0x8;

    #[repr(C)]
    struct SockAddr {
        family: u16,
        data: [u8; 14],
    }

    #[repr(C)]
    struct InAddr {
        address: u32,
    }

    #[repr(C)]
    struct SockAddrIn {
        family: u16,
        port: u16,
        address: InAddr,
        zero: [u8; 8],
    }

    #[repr(C)]
    struct IfAddrs {
        next: *mut IfAddrs,
        name: *mut c_char,
        flags: c_uint,
        address: *mut SockAddr,
        netmask: *mut SockAddr,
        ifu: *mut SockAddr,
        data: *mut c_void,
    }

    unsafe extern "C" {
        fn getifaddrs(addresses: *mut *mut IfAddrs) -> c_int;
        fn freeifaddrs(addresses: *mut IfAddrs);
    }

    let mut first = ptr::null_mut();
    // SAFETY: `first` is a valid out-pointer and is released with
    // `freeifaddrs` on every successful call.
    if unsafe { getifaddrs(&raw mut first) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut rows = Vec::new();
    let mut current = first;
    while !current.is_null() {
        // SAFETY: nodes in the list returned by `getifaddrs` remain valid
        // until `freeifaddrs(first)`.
        let interface = unsafe { &*current };
        if !interface.address.is_null()
            // SAFETY: the non-null pointer addresses at least a `sockaddr`.
            && unsafe { (*interface.address).family } == AF_INET
            && !interface.name.is_null()
        {
            // SAFETY: `ifa_name` is a NUL-terminated string owned by the list.
            let name = unsafe { CStr::from_ptr(interface.name) }
                .to_string_lossy()
                .into_owned();
            // SAFETY: AF_INET guarantees the address points to `sockaddr_in`.
            let address = unsafe { &*(interface.address.cast::<SockAddrIn>()) };
            let ip = Ipv4Addr::from(address.address.address.to_ne_bytes());
            let mac = std::fs::read_to_string(format!("/sys/class/net/{name}/address"))
                .unwrap_or_default()
                .trim()
                .to_owned();
            rows.push(NetworkInterfaceRow {
                name,
                address: ip,
                mac,
                internal: interface.flags & IFF_LOOPBACK != 0,
            });
        }
        current = interface.next;
    }
    // SAFETY: `first` came from a successful `getifaddrs` invocation.
    unsafe { freeifaddrs(first) };
    Ok(rows)
}

#[cfg(target_os = "macos")]
fn macos_interfaces() -> io::Result<Vec<NetworkInterfaceRow>> {
    let output = Command::new("ifconfig").arg("-a").output()?;
    if !output.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut rows = Vec::new();
    let mut name = String::new();
    let mut mac = String::new();
    let mut internal = false;
    for line in text.lines() {
        if line
            .chars()
            .next()
            .is_some_and(|character| !character.is_whitespace())
        {
            name = line.split(':').next().unwrap_or_default().to_owned();
            mac.clear();
            internal = line.contains("<LOOPBACK,") || line.contains(",LOOPBACK,");
        } else {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.first() == Some(&"ether") {
                mac = fields.get(1).copied().unwrap_or_default().to_owned();
            } else if fields.first() == Some(&"inet") {
                if let Some(address) = fields.get(1).and_then(|value| value.parse().ok()) {
                    rows.push(NetworkInterfaceRow {
                        name: name.clone(),
                        address,
                        mac: mac.clone(),
                        internal,
                    });
                }
            }
        }
    }
    Ok(rows)
}

pub async fn get_local_network_ip() -> Option<Ipv4Addr> {
    get_local_network_ip_with(&SystemNetworkInterfaceProvider).await
}

pub async fn get_local_network_ip_with(
    interfaces: &dyn NetworkInterfaceProvider,
) -> Option<Ipv4Addr> {
    let ip = probe_default_route_ipv4().await.ok()?;
    let rows = interfaces.interfaces().ok()?;
    filter_local_network_ip(ip, &rows)
}

pub type LanIpError = Box<dyn Error + Send + Sync>;
pub type LanIpFuture = Pin<Box<dyn Future<Output = Result<Option<Ipv4Addr>, LanIpError>> + Send>>;

pub trait LanIpResolver: Send + Sync {
    fn resolve(&self) -> LanIpFuture;
}

impl<F, Fut, E> LanIpResolver for F
where
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = Result<Option<Ipv4Addr>, E>> + Send + 'static,
    E: Error + Send + Sync + 'static,
{
    fn resolve(&self) -> LanIpFuture {
        let future = self();
        Box::pin(async move { future.await.map_err(|error| Box::new(error) as LanIpError) })
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemLanIpResolver;

impl LanIpResolver for SystemLanIpResolver {
    fn resolve(&self) -> LanIpFuture {
        Box::pin(async { Ok(get_local_network_ip().await) })
    }
}

pub struct LanIpMonitorOptions {
    pub initial_ip: Option<Ipv4Addr>,
    pub interval: Duration,
    pub resolver: Arc<dyn LanIpResolver>,
    pub on_change: LanIpChangeCallback,
    pub on_error: Option<LanIpErrorCallback>,
}

pub type LanIpChangeCallback = Arc<dyn Fn(Option<Ipv4Addr>, Option<Ipv4Addr>) + Send + Sync>;
pub type LanIpErrorCallback = Arc<dyn Fn(&dyn Error) + Send + Sync>;
pub type MdnsErrorCallback = Arc<dyn Fn(String) + Send + Sync>;

pub struct LanIpMonitor {
    task: JoinHandle<()>,
}

impl LanIpMonitor {
    pub fn stop(self) {
        self.task.abort();
    }
}

impl Drop for LanIpMonitor {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub fn start_lan_ip_monitor(options: LanIpMonitorOptions) -> LanIpMonitor {
    let task = tokio::spawn(async move {
        let mut current_ip = options.initial_ip;
        loop {
            tokio::time::sleep(options.interval).await;
            match options.resolver.resolve().await {
                Ok(next_ip) if next_ip != current_ip => {
                    let previous_ip = current_ip;
                    current_ip = next_ip;
                    (options.on_change)(next_ip, previous_ip);
                }
                Ok(_) => {}
                Err(error) => {
                    if let Some(on_error) = &options.on_error {
                        on_error(error.as_ref());
                    }
                }
            }
        }
    });
    LanIpMonitor { task }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MdnsPlatform {
    MacOs,
    Linux,
    Unsupported,
}

impl MdnsPlatform {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Unsupported
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MdnsPublisherSpec {
    pub command: &'static str,
    pub probe_args: &'static [&'static str],
    pub missing_reason: &'static str,
}

pub fn mdns_publisher(platform: MdnsPlatform) -> Option<MdnsPublisherSpec> {
    match platform {
        MdnsPlatform::MacOs => Some(MdnsPublisherSpec {
            command: "dns-sd",
            probe_args: &["-h"],
            missing_reason: "dns-sd not found",
        }),
        MdnsPlatform::Linux => Some(MdnsPublisherSpec {
            command: "avahi-publish-address",
            probe_args: &["--help"],
            missing_reason: "avahi-publish-address not found. Install avahi-utils: sudo apt \
                             install avahi-utils",
        }),
        MdnsPlatform::Unsupported => None,
    }
}

pub fn build_publish_args(
    platform: MdnsPlatform,
    fqdn: &str,
    name: &str,
    port: u16,
    ip: Ipv4Addr,
) -> Vec<String> {
    match platform {
        MdnsPlatform::MacOs => vec![
            "-P".into(),
            name.into(),
            "_http._tcp".into(),
            "local".into(),
            port.to_string(),
            fqdn.into(),
            ip.to_string(),
        ],
        MdnsPlatform::Linux => vec!["-R".into(), fqdn.into(), ip.to_string()],
        MdnsPlatform::Unsupported => Vec::new(),
    }
}

pub trait MdnsCommandRunner: Send + Sync {
    fn command_exists(&self, command: &str, probe_args: &[&str]) -> bool;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemMdnsCommandRunner;

impl MdnsCommandRunner for SystemMdnsCommandRunner {
    fn command_exists(&self, command: &str, probe_args: &[&str]) -> bool {
        let spawned = Command::new(command)
            .args(probe_args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(error) => return error.kind() != io::ErrorKind::NotFound,
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        loop {
            match child.try_wait() {
                Ok(Some(_)) | Err(_) => return true,
                Ok(None) if std::time::Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return true;
                }
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MdnsSupport {
    pub supported: bool,
    pub reason: Option<String>,
}

pub fn is_mdns_supported() -> MdnsSupport {
    is_mdns_supported_with(MdnsPlatform::current(), &SystemMdnsCommandRunner)
}

pub fn is_mdns_supported_with(
    platform: MdnsPlatform,
    runner: &dyn MdnsCommandRunner,
) -> MdnsSupport {
    let Some(publisher) = mdns_publisher(platform) else {
        return MdnsSupport {
            supported: false,
            reason: Some("mDNS publishing is not supported on this platform".into()),
        };
    };
    if !runner.command_exists(publisher.command, publisher.probe_args) {
        return MdnsSupport {
            supported: false,
            reason: Some(publisher.missing_reason.into()),
        };
    }
    MdnsSupport {
        supported: true,
        reason: None,
    }
}

pub trait MdnsChild: Send {
    fn id(&self) -> u32;
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>>;
    fn terminate(&mut self) -> io::Result<()>;
    fn reap(&mut self) {}
}

impl MdnsChild for Child {
    fn id(&self) -> u32 {
        Child::id(self)
    }

    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        Child::try_wait(self)
    }

    fn terminate(&mut self) -> io::Result<()> {
        terminate_pid(self.id())
    }

    fn reap(&mut self) {
        let _ = self.wait();
    }
}

pub trait MdnsSpawner: Send + Sync {
    fn spawn(&self, command: &str, args: &[String]) -> io::Result<Box<dyn MdnsChild>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemMdnsSpawner;

impl MdnsSpawner for SystemMdnsSpawner {
    fn spawn(&self, command: &str, args: &[String]) -> io::Result<Box<dyn MdnsChild>> {
        Command::new(command)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|child| Box::new(child) as Box<dyn MdnsChild>)
    }
}

fn terminate_pid(pid: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        let status = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other("failed to send SIGTERM"))
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "SIGTERM is unsupported on this platform",
        ))
    }
}

#[derive(Debug)]
pub struct MdnsError(pub String);

impl fmt::Display for MdnsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for MdnsError {}

struct ActivePublisher {
    id: u64,
    stop: mpsc::Sender<()>,
}

pub struct MdnsManager {
    platform: MdnsPlatform,
    spawner: Arc<dyn MdnsSpawner>,
    active: Arc<Mutex<HashMap<String, ActivePublisher>>>,
    next_id: AtomicU64,
}

impl MdnsManager {
    pub fn new(platform: MdnsPlatform, spawner: Arc<dyn MdnsSpawner>) -> Self {
        Self {
            platform,
            spawner,
            active: Arc::new(Mutex::new(HashMap::new())),
            next_id: AtomicU64::new(1),
        }
    }

    pub fn publish(
        &self,
        hostname: &str,
        port: u16,
        ip: Ipv4Addr,
        on_error: Option<MdnsErrorCallback>,
    ) -> Result<(), MdnsError> {
        let mut active = lock(&self.active);
        if active.contains_key(hostname) {
            return Ok(());
        }
        let Some(publisher) = mdns_publisher(self.platform) else {
            return Ok(());
        };
        let fqdn = if hostname.ends_with(".local") {
            hostname.to_owned()
        } else {
            format!("{hostname}.local")
        };
        let name = fqdn.strip_suffix(".local").unwrap_or(&fqdn);
        let args = build_publish_args(self.platform, &fqdn, name, port, ip);
        let mut child = self
            .spawner
            .spawn(publisher.command, &args)
            .map_err(|error| {
                let message = if error.kind() == io::ErrorKind::NotFound {
                    publisher.missing_reason.to_owned()
                } else {
                    format!("mDNS publish error for {hostname}: {error}")
                };
                if let Some(callback) = &on_error {
                    callback(message.clone());
                }
                MdnsError(message)
            })?;
        let (stop, stop_receiver) = mpsc::channel();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        active.insert(hostname.to_owned(), ActivePublisher { id, stop });
        drop(active);

        let active = Arc::clone(&self.active);
        let hostname_owned = hostname.to_owned();
        thread::spawn(move || {
            loop {
                if stop_receiver.try_recv().is_ok() {
                    let _ = child.terminate();
                    child.reap();
                    break;
                }
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) => thread::sleep(Duration::from_millis(25)),
                    Err(error) => {
                        if let Some(callback) = &on_error {
                            callback(format!("mDNS publish error for {hostname_owned}: {error}"));
                        }
                        break;
                    }
                }
            }
            let mut map = lock(&active);
            if map.get(&hostname_owned).is_some_and(|entry| entry.id == id) {
                map.remove(&hostname_owned);
            }
        });
        Ok(())
    }

    pub fn unpublish(&self, hostname: &str) {
        if let Some(entry) = lock(&self.active).remove(hostname) {
            let _ = entry.stop.send(());
        }
    }

    pub fn cleanup_all(&self) {
        let publishers = std::mem::take(&mut *lock(&self.active));
        for entry in publishers.into_values() {
            let _ = entry.stop.send(());
        }
    }

    pub fn published(&self) -> Vec<String> {
        let mut entries = lock(&self.active)
            .iter()
            .map(|(hostname, publisher)| (publisher.id, hostname.clone()))
            .collect::<Vec<_>>();
        entries.sort_unstable_by_key(|(id, _)| *id);
        entries.into_iter().map(|(_, hostname)| hostname).collect()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn default_manager() -> &'static MdnsManager {
    static MANAGER: OnceLock<MdnsManager> = OnceLock::new();
    MANAGER.get_or_init(|| MdnsManager::new(MdnsPlatform::current(), Arc::new(SystemMdnsSpawner)))
}

pub fn publish(
    hostname: &str,
    port: u16,
    ip: Ipv4Addr,
    on_error: Option<MdnsErrorCallback>,
) -> Result<(), MdnsError> {
    default_manager().publish(hostname, port, ip, on_error)
}

pub fn unpublish(hostname: &str) {
    default_manager().unpublish(hostname);
}

pub fn cleanup_all() {
    default_manager().cleanup_all();
}

pub fn get_published() -> Vec<String> {
    default_manager().published()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    use super::*;

    fn row(name: &str, mac: &str, internal: bool) -> NetworkInterfaceRow {
        NetworkInterfaceRow {
            name: name.into(),
            address: "192.168.1.10".parse().expect("test IP"),
            mac: mac.into(),
            internal,
        }
    }

    #[test]
    fn filters_internal_and_virtual_interfaces() {
        let ip = "192.168.1.10".parse().expect("test IP");
        assert_eq!(
            filter_local_network_ip(ip, &[row("en0", "aa:bb:cc:dd:ee:ff", false)]),
            Some(ip)
        );
        for interface in [
            row("en0", "00:00:00:00:00:00", false),
            row("eth0", "00:15:5d:01:02:03", false),
            row("vEthernet (Default Switch)", "aa:bb:cc:dd:ee:ff", false),
            row("bridge0", "aa:bb:cc:dd:ee:ff", false),
            row("en0", "aa:bb:cc:dd:ee:ff", true),
        ] {
            assert_eq!(filter_local_network_ip(ip, &[interface]), None);
        }
        assert_eq!(filter_local_network_ip(Ipv4Addr::LOCALHOST, &[]), None);
    }

    #[tokio::test]
    async fn monitor_reports_loss_recovery_and_stops() {
        let values = Arc::new(Mutex::new(vec![
            Some("192.168.1.77".parse().expect("test IP")),
            None,
            Some("192.168.1.99".parse().expect("test IP")),
        ]));
        let resolver_values = Arc::clone(&values);
        let resolver = move || {
            let value = lock(&resolver_values).remove(0);
            async move { Ok::<_, io::Error>(value) }
        };
        let changes = Arc::new(Mutex::new(Vec::new()));
        let callback_changes = Arc::clone(&changes);
        let monitor = start_lan_ip_monitor(LanIpMonitorOptions {
            initial_ip: Some("192.168.1.42".parse().expect("test IP")),
            interval: Duration::from_millis(5),
            resolver: Arc::new(resolver),
            on_change: Arc::new(move |next, previous| {
                lock(&callback_changes).push((next, previous));
            }),
            on_error: None,
        });
        for _ in 0..3 {
            tokio::time::sleep(Duration::from_millis(7)).await;
        }
        monitor.stop();
        assert_eq!(lock(&changes).len(), 3);
    }

    #[test]
    fn publisher_builds_platform_commands() {
        assert_eq!(
            build_publish_args(
                MdnsPlatform::MacOs,
                "api.app.local",
                "api.app",
                443,
                "192.168.1.10".parse().expect("test IP")
            ),
            vec![
                "-P",
                "api.app",
                "_http._tcp",
                "local",
                "443",
                "api.app.local",
                "192.168.1.10"
            ]
        );
        assert_eq!(
            build_publish_args(
                MdnsPlatform::Linux,
                "app.local",
                "app",
                443,
                "192.168.1.10".parse().expect("test IP")
            ),
            vec!["-R", "app.local", "192.168.1.10"]
        );
    }

    struct FakeChild {
        terminated: Arc<AtomicBool>,
    }

    impl MdnsChild for FakeChild {
        fn id(&self) -> u32 {
            42
        }

        fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
            Ok(None)
        }

        fn terminate(&mut self) -> io::Result<()> {
            self.terminated.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    struct FakeSpawner {
        calls: Arc<Mutex<SpawnCalls>>,
        terminated: Arc<AtomicBool>,
        count: AtomicUsize,
    }

    type SpawnCalls = Vec<(String, Vec<String>)>;

    impl MdnsSpawner for FakeSpawner {
        fn spawn(&self, command: &str, args: &[String]) -> io::Result<Box<dyn MdnsChild>> {
            self.count.fetch_add(1, Ordering::SeqCst);
            lock(&self.calls).push((command.into(), args.to_vec()));
            Ok(Box::new(FakeChild {
                terminated: Arc::clone(&self.terminated),
            }))
        }
    }

    #[test]
    fn manager_deduplicates_and_terminates_publishers() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let terminated = Arc::new(AtomicBool::new(false));
        let spawner = Arc::new(FakeSpawner {
            calls: Arc::clone(&calls),
            terminated: Arc::clone(&terminated),
            count: AtomicUsize::new(0),
        });
        let manager = MdnsManager::new(MdnsPlatform::Linux, spawner.clone());
        let ip = "192.168.1.10".parse().expect("test IP");
        manager
            .publish("app", 443, ip, None)
            .expect("publish should pass");
        manager
            .publish("app", 443, ip, None)
            .expect("duplicate should pass");
        assert_eq!(spawner.count.load(Ordering::SeqCst), 1);
        assert_eq!(manager.published(), vec!["app"]);
        assert_eq!(
            lock(&calls)[0],
            (
                "avahi-publish-address".into(),
                vec!["-R".into(), "app.local".into(), "192.168.1.10".into()]
            )
        );
        manager.unpublish("app");
        for _ in 0..20 {
            if terminated.load(Ordering::SeqCst) {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(terminated.load(Ordering::SeqCst));
    }
}

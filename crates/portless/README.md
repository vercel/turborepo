# portless

Stable, named local-development URLs without port numbers.

Portless runs development servers behind a local reverse proxy, so a command
such as:

```console
$ portless myapp next dev
https://myapp.localhost
```

keeps the same URL when the application's port changes. This package contains
both the `portless` command and a Rust library suitable for embedding in
Turborepo and other developer tools.

## Status and provenance

This is a Rust port of
[`vercel-labs/portless` 0.15.1](https://github.com/vercel-labs/portless/tree/7c9be3c7d76c34f68f10dbb8072b6833a9b114ac),
based on the exact upstream commit
[`7c9be3c7d76c34f68f10dbb8072b6833a9b114ac`](https://github.com/vercel-labs/portless/commit/7c9be3c7d76c34f68f10dbb8072b6833a9b114ac).
The state format and command behavior intentionally remain compatible with
that release. The Rust API is native to this package.

## Requirements

- Rust 1.88 or newer to build.
- macOS, Linux, or Windows. Core proxying works on all three; service and
  network integration use each platform's native facilities.
- OpenSSL on `PATH` when generating local certificates.
- Permission to bind the selected proxy ports. Ports 80 and 443 commonly
  require elevation.

Optional runtime integrations have additional requirements:

- Tailscale Serve or Funnel requires an installed, authenticated `tailscale`
  CLI.
- Public tunnels require an installed, authenticated `ngrok` CLI.
- LAN `.local` publication uses `dns-sd` on macOS or
  `avahi-publish-address` (usually from `avahi-utils`) on Linux. Native mDNS
  publication is not available on Windows.
- Certificate trust uses Keychain's `security` on macOS, the distribution's
  CA update command on Linux, and `certutil` on Windows.
- Background services use launchd on macOS, systemd on Linux, and Task
  Scheduler on Windows.

Git and Node.js package managers are detected when project/workspace inference
needs them, but they are not required by the proxy library itself.

## Install and build

Install the command from a checkout of the Turborepo repository:

```console
cargo install --path crates/portless --locked
```

Build the package in the workspace:

```console
cargo build -p portless
```

To consume the library from the same checkout:

```toml
[dependencies]
portless = { path = "../turborepo/crates/portless" }
```

## Command-line use

Run a named application:

```console
portless myapp npm run dev
```

Infer the project name and development command:

```console
portless
portless run
```

Common management commands are:

```text
portless get <name>
portless alias <name> <port>
portless list
portless doctor
portless trust
portless clean
portless prune [--force]
portless hosts sync|clean
portless proxy start|stop
portless service install|status|uninstall
```

Use `portless --help`, `portless run --help`, or
`portless proxy --help` for the complete option reference.

Configuration can live in `portless.json` or in the `portless` field of
`package.json`. It controls names, scripts, app ports, proxy opt-out, workspace
applications, and Turbo mode. The principal environment variables are
`PORTLESS_PORT`, `PORTLESS_APP_PORT`, `PORTLESS_HTTPS`, `PORTLESS_LAN`,
`PORTLESS_LAN_IP`, `PORTLESS_TLD`, `PORTLESS_WILDCARD`,
`PORTLESS_SYNC_HOSTS`, `PORTLESS_TAILSCALE`, `PORTLESS_FUNNEL`,
`PORTLESS_NGROK`, and `PORTLESS_STATE_DIR`.

## Library use

The crate root re-exports the route registry and proxy types needed for the
usual embedding path. Detailed integration APIs remain available in the
documented public modules.

```rust,no_run
use std::{path::PathBuf, sync::Arc};

use portless::{ProxyOptions, ProxyServer, RouteStore};

async fn serve(state_dir: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let routes = Arc::new(RouteStore::new(state_dir));
    routes.ensure_dir()?;

    let route_source = Arc::clone(&routes);
    let options = ProxyOptions::new(0, move || route_source.load_routes());
    let server = ProxyServer::bind(options).await?;

    println!("listening on {}", server.local_addr()?);
    server.run().await?;
    Ok(())
}
```

`ProxyOptions::new` defaults to strict hostname matching, `.localhost`, plain
HTTP, and an error handler that writes to stderr. Supplying TLS material also
enables best-effort HTTP-to-HTTPS redirects on port 80. Configure its public
fields before binding when embedding the proxy.

## Features and integrations

This package defines no Cargo feature flags: the library and command expose
the same compiled capabilities in every build. HTTPS, LAN mode, wildcard
routing, host-file synchronization, Tailscale, Funnel, ngrok, and background
services are selected at runtime through command options, configuration, or
environment variables. Optional external tools are invoked only when their
corresponding behavior is requested.

The library modules separate:

- proxy serving and disk-backed route storage;
- local CA creation, host certificates, and trust-store operations;
- configuration, project-name inference, process setup, and workspace
  discovery;
- hosts-file, LAN address, mDNS, Tailscale, ngrok, and service management;
- Turborepo development-manifest support and generated status pages.

## State

By default, per-user state is stored under `~/.portless`; set
`PORTLESS_STATE_DIR` to use another location. State includes `routes.json`,
proxy PID/port markers, TLS keys and certificates, generated host
certificates, service metadata, and the Turborepo development manifest.
Portless also recognizes the legacy temporary state directory used by
upstream 0.15.1.

Route writes are lock-protected and use the upstream-compatible JSON schema.
`portless clean` removes known Portless artifacts, while `portless prune`
removes stale routes and processes. Neither command is a general-purpose
directory cleaner.

## Security

Portless is a development tool, not an internet-facing authentication layer.
Keep the proxy and state directory limited to trusted users and networks.

- The local CA private key is sensitive. Do not commit, copy, or share the
  state directory.
- `portless trust`, hosts-file changes, privileged ports, and service
  installation can modify system configuration and may request elevation.
  Inspect the requested operation before approving it.
- LAN and wildcard modes broaden who can reach development servers.
- Tailscale Funnel and ngrok can make a local service publicly reachable.
  Applications exposed this way must provide their own authentication and
  authorization.
- Routes do not add access control to the proxied application.

## License

Copyright 2025 Vercel Inc.

Licensed under the Apache License, Version 2.0. See the
[full license text](https://github.com/vercel/turborepo/blob/main/crates/portless/LICENSE).


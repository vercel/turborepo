# Local OTEL collector example

This example shows how to build the `turbo` binary with the **OpenTelemetry (OTEL)** feature enabled and send metrics to a local collector running via **Docker Compose**. It’s intended as a lightweight, local integration harness for the OTEL exporter.

## 1. Prerequisites

- **Docker & docker compose** installed and running
- **Rust toolchain** (matching this repo’s `rust-toolchain.toml`)
- **pnpm** installed (per `CONTRIBUTING.md`)

All commands below assume the repo root is `turborepo/`.

## 2. Build `turbo` with the OTEL feature

From the repo root:

```bash
pnpm install
cargo build -p turbo --features otel
```

This produces an OTEL-enabled `turbo` binary at: `./target/debug/turbo`

## 3. Start the local collector stack

From this example directory:

```bash
cd crates/turborepo-otel/examples/local-collector
docker compose up -d
```

This starts:

- **`otel-collector`** (OTLP receiver + debug + Prometheus exporter)
- **`prometheus`** (scrapes metrics from the collector)
- **`grafana`** (optional visualization)

Ports:

- **OTLP gRPC**: `4317`
- **OTLP HTTP**: `4318`
- **Collector metrics / Prometheus exporter**: `8888`, `8889`
- **Prometheus UI**: `9090`
- **Grafana UI**: `3000`

You can confirm the collector is ready:

```bash
docker compose logs otel-collector
```

You should see a line similar to:

```text
Everything is ready. Begin running and processing data.
```

## 4. Configure `turbo` to send metrics to the local collector

In a new shell, from the repo root, export the OTEL env vars:

```bash
cd /path/to/turborepo

export TURBO_EXPERIMENTAL_OTEL_ENABLED=1
export TURBO_EXPERIMENTAL_OTEL_PROTOCOL=grpc
export TURBO_EXPERIMENTAL_OTEL_ENDPOINT=http://127.0.0.1:4317
export TURBO_EXPERIMENTAL_OTEL_RESOURCE="service.name=turborepo,env=local"
# Optional (defaults shown)
export TURBO_EXPERIMENTAL_OTEL_METRICS_RUN_SUMMARY=1
export TURBO_EXPERIMENTAL_OTEL_METRICS_TASK_DETAILS=0
```

These environment variables bypass `turbo.json` and directly configure the OTEL exporter.

## 5. Run a task and emit metrics

Use the **locally built** OTEL-enabled binary:

```bash
./target/debug/turbo run lint --filter=turbo-ignore
```

You can replace `lint --filter=turbo-ignore` with any real task in this repo; the important part is that the command finishes so a run summary can be exported.

## 6. Verify metrics reached the collector

- **Collector logs (debug exporter)**:

  ```bash
  cd crates/turborepo-otel/examples/local-collector
  docker compose logs --tail=100 otel-collector
  ```

  You should see entries like:

  ```text
  Metrics {"otelcol.component.id": "debug", "otelcol.signal": "metrics", "resource metrics": 1, "metrics": 4, "data points": 4}
  Resource attributes:
       -> env: Str(local)
       -> service.name: Str(turborepo)
  Metric #0
       -> Name: turbo.run.duration_ms
  Metric #1
       -> Name: turbo.run.tasks.attempted
  Metric #2
       -> Name: turbo.run.tasks.failed
  Metric #3
       -> Name: turbo.run.tasks.cached
  ```

- **Prometheus UI** (optional):

  - Open `http://localhost:9090`
  - Query for metrics such as:
    - `turbo.run.duration_ms`
    - `turbo.run.tasks.attempted`
    - `turbo.run.tasks.failed`
    - `turbo.run.tasks.cached`

- **Grafana UI** (optional):

  - Open `http://localhost:3000` (default credentials are usually `admin` / `admin`)
  - Add a Prometheus data source pointing at `http://prometheus:9090`
  - Build dashboards using the `turbo.*` metrics

## 7. Cleanup

To stop the local collector stack:

```bash
cd crates/turborepo-otel/examples/local-collector
docker compose down
```

The OTEL-enabled `turbo` binary remains available at `./target/debug/turbo`. You can continue using it with the same environment variables to send metrics to this collector or to another OTLP-compatible backend.

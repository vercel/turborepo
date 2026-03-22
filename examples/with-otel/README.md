# Turborepo with OpenTelemetry

This example shows a Turborepo monorepo with a local [OpenTelemetry Collector](https://opentelemetry.io/docs/collector/), [Prometheus](https://prometheus.io/), and [Grafana](https://grafana.com/) for visualizing Turborepo's OTEL metrics.

## Using this example

Run the following command:

```sh
npx create-turbo@latest -e with-otel
```

## What's inside?

This Turborepo includes the following packages/apps:

### Apps and Packages

- `docs`: a [Next.js](https://nextjs.org/) app
- `web`: another [Next.js](https://nextjs.org/) app
- `@repo/ui`: a stub React component library shared by both `web` and `docs` applications
- `@repo/eslint-config`: `eslint` configurations (includes `eslint-config-next` and `eslint-config-prettier`)
- `@repo/typescript-config`: `tsconfig.json`s used throughout the monorepo

Each package/app is 100% [TypeScript](https://www.typescriptlang.org/).

### Utilities

This Turborepo has some additional tools already setup for you:

- [TypeScript](https://www.typescriptlang.org/) for static type checking
- [ESLint](https://eslint.org/) for code linting
- [Prettier](https://prettier.io) for code formatting

### OTEL Collector Stack

The included `docker-compose.yml` starts:

- **`otel-collector`** -- OTLP gRPC receiver with a debug exporter and Prometheus exporter
- **`prometheus`** -- scrapes metrics from the collector
- **`grafana`** -- pre-configured with a Turborepo dashboard (anonymous access enabled)

| Service              | Port  |
| -------------------- | ----- |
| OTLP gRPC            | 4317  |
| Collector metrics    | 8888  |
| Prometheus exporter  | 8889  |
| Prometheus UI        | 9090  |
| Grafana UI           | 3001  |

## How to use

### 1. Start the collector stack

```sh
docker compose up -d
```

Confirm it's ready:

```sh
docker compose logs otel-collector
# Look for: "Everything is ready. Begin running and processing data."
```

### 2. Configure Turborepo OTEL environment variables

**macOS / Linux:**

```sh
export TURBO_EXPERIMENTAL_OTEL_ENABLED=1
export TURBO_EXPERIMENTAL_OTEL_ENDPOINT=https://127.0.0.1:4317
export TURBO_EXPERIMENTAL_OTEL_RESOURCE="service.name=turborepo,env=local"
export TURBO_EXPERIMENTAL_OTEL_METRICS_TASK_DETAILS=1
```

**Windows (PowerShell):**

```powershell
$env:TURBO_EXPERIMENTAL_OTEL_ENABLED=1
$env:TURBO_EXPERIMENTAL_OTEL_ENDPOINT="https://127.0.0.1:4317"
$env:TURBO_EXPERIMENTAL_OTEL_RESOURCE="service.name=turborepo,env=local"
$env:TURBO_EXPERIMENTAL_OTEL_METRICS_TASK_DETAILS=1
```

### 3. Run a task

```sh
turbo build
```

### 4. Verify metrics

**Collector logs (debug exporter)**:

```sh
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

**Prometheus UI**: Click any of these links to open pre-filled queries:

- [All run metrics](http://localhost:9090/graph?g0.expr=%7B__name__%3D~%22turbo_run_.%2B%22%7D&g0.tab=0) -- `{__name__=~"turbo_run_.+"}`
- [Run duration (histogram)](http://localhost:9090/graph?g0.expr=turbo_run_duration_ms_sum+%2F+turbo_run_duration_ms_count&g0.tab=0) -- avg duration per run
- [Tasks attempted](http://localhost:9090/graph?g0.expr=turbo_run_tasks_attempted_total&g0.tab=0)
- [Tasks cached](http://localhost:9090/graph?g0.expr=turbo_run_tasks_cached_total&g0.tab=0)
- [Tasks failed](http://localhost:9090/graph?g0.expr=turbo_run_tasks_failed_total&g0.tab=0)
- [Cache hit rate](http://localhost:9090/graph?g0.expr=turbo_run_tasks_cached_total+%2F+clamp_min(turbo_run_tasks_attempted_total%2C+1)&g0.tab=0)

**Grafana dashboard**: Open `http://localhost:3001` -- the **Turborepo Runs** dashboard is pre-configured and loads automatically. No login required. The dashboard includes:

- Run duration (avg and p95) and runs over time
- Tasks attempted, cached, failed, and cache hit rate
- **Task breakdown** -- duration by task (build, lint, check-types, etc.), cache status by task, a detail table with package names, and time-series charts for tracking changes across runs

### 5. Cleanup

```sh
docker compose down
```

## Useful Links

Learn more about the power of Turborepo:

- [Tasks](https://turborepo.dev/docs/crafting-your-repository/running-tasks)
- [Caching](https://turborepo.dev/docs/crafting-your-repository/caching)
- [Remote Caching](https://turborepo.dev/docs/core-concepts/remote-caching)
- [Filtering](https://turborepo.dev/docs/crafting-your-repository/running-tasks#using-filters)
- [Configuration Options](https://turborepo.dev/docs/reference/configuration)
- [CLI Usage](https://turborepo.dev/docs/reference/command-line-reference)

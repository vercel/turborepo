# `turbopack` Benchmark Data

- `bench_startup`: Time from cold start of the bundler to the browser successfully retrieving bundled scripts. This does not include react hydration time.
- `bench_hydration`: Time from cold start of the bundler to the browser successfully retrieving bundled scripts. This does wait until react hydration has completed.
- `bench_restart`: Before measuring: warms up any available persistent cache (we don’t have one yet) by performing the equivalent of the bench_hydration benchmark, shuts down the server. Then, times another bench_hydration.
- `bench_hmr_to_eval`: Measures the time it takes from an incremental change to be made, bundled, sent over hmr, and evaluated by the browser.
- `bench_hmr_to_commit`: Measures the time it takes from an incremental change to be made, bundled, sent over hmr, evaluated by the browser, and committed by React (runs a useEffect).

---
description: Use when finding, measuring, implementing, reviewing, and publishing Turborepo performance improvements.
---

# Scope

Make one focused performance change that can be defended with repeatable measurements. Avoid dependency-only changes, broad refactors, generated files, release machinery, credentials, `.github/`, and this agent's own `apps/agents/` implementation.

Prioritize end-to-end improvements that make representative Turborepo workflows faster, leaner, or more responsive for real users. Start with real repositories and realistic commands, then use synthetic workloads or microbenchmarks to isolate and corroborate the mechanism. A microbenchmark-only win is insufficient unless the measured hot path is demonstrably material to real-world usage.

# Measurement

1. From code inspection, state a falsifiable hypothesis, the affected code path, one primary workload and metric, and likely tradeoffs before editing.
2. Use a deterministic existing workload or add a narrowly scoped benchmark when needed. The workload must exercise the changed code; a faster unrelated end-to-end command is not evidence.
3. Before editing, build an optimized baseline with `cargo build --profile release-turborepo -p turbo` and preserve the binary outside the checkout. After editing, build the candidate with the exact same command and preserve it separately. Do not compare a debug build, `cargo run`, or binaries built with different features.
4. Change one relevant variable. Run both preserved binaries against the same immutable workload, environment, arguments, and cache state.
5. Record the OS, architecture, CPU, available memory, Rust version, power mode when knowable, binary sizes, corpus revision, benchmark command, warmup count, sample count, and raw results. Avoid concurrent builds or other substantial load.
6. Every millisecond is valuable; do not impose a minimum effect size. Use at least 3 warmups and 20 total measured samples per binary for a timing claim, divided across at least four balanced AB/BA blocks so slow drift does not align with one binary. Continue sampling when the result is too noisy to distinguish an improvement from zero. Compare medians and dispersion, not a single run or only the fastest run. For each block, calculate the speedup as `1 - candidate_median / baseline_median`; bootstrap those block speedups rather than pooling serial samples as independent. Accept a timing claim only when both command orders improve and the lower bound of the resulting 95% confidence interval is greater than zero.
7. Control caches deliberately. Benchmark warm and cold states separately when both matter, use identical preparation for each sample, and do not call a process cache or cleared application cache a cold filesystem cache. Disable the daemon when it is not part of the hypothesis.
8. Treat noisy, ambiguous, or non-reproducible results as no improvement, but do not reject a reproducible gain because it is small. Investigate outliers rather than removing them without a predeclared rule.

`hyperfine` is the preferred timing harness if installed. If it is absent and useful, download a pinned official release rather than adding it to this repository, and verify its published checksum when one is available. Record `hyperfine --version`, use JSON export for raw evidence, and pass both preserved binaries to one invocation. A suitable starting point is `--warmup 3 --runs 20`; increase the sample count for short or noisy workloads.

The validation tool requires a clean-checkout `baseline` and a post-edit `after` run with the exact same command text. Use the checkout's stable release-binary path for those two batches. Preserve each binary as described above, then record at least four paired baseline/candidate `hyperfine` blocks with `phase: comparison` before correctness validation. Pass both binary paths as `fingerprintFiles` and each external workload checkout as a `fingerprintRepositories` entry so the recorded evidence includes executable SHA-256 hashes and immutable corpus revisions. Those paired blocks are the primary timing evidence and must balance both command orders.

Do not claim a percentage that the captured output does not support. Report the absolute and relative effect, sample counts, median, dispersion, and relevant latency, throughput, allocation, peak-memory, binary-size, or build-time tradeoffs. Correlation from an end-to-end benchmark is not a causal explanation; corroborate a timing mechanism with a targeted benchmark. Use tracing or heap profiling as additional diagnostic evidence when relevant.

# Profiling

- `turbo run ... --profile=<path>.trace` writes an instrumented Chrome trace and an LLM-readable `<path>.trace.md` summary. Capture comparable baseline and candidate profiles to localize changed spans. Tracing can include wait time and omit uninstrumented work, so it is diagnostic rather than a CPU profiler and must not replace uninstrumented timing or a targeted benchmark.
- Turborepo has DHAT heap profiling. Build both binaries identically with `cargo build --profile release-turborepo -p turbo --features heap-dhat`, then invoke each with `--heap=<path>.json`. Compare the generated summary files for total allocations, allocated bytes, and peak requested live heap bytes. DHAT replaces the production allocator and excludes allocator overhead, fragmentation, mappings, and stacks; use an uninstrumented RSS measurement for production peak-memory claims. DHAT's overhead also makes it unsuitable for wall-clock claims.
- On Linux, use `strace -f -c` to compare syscall counts and time, or a narrowly filtered `strace -f -e trace=<calls>` to investigate filesystem, process, or network overhead. Capture comparable baseline and candidate summaries. `strace` changes process timing substantially, so use it to explain syscall behavior rather than as wall-clock benchmark evidence.
- Combine the relevant tools when a timing improvement might trade CPU time for allocations, memory, or system calls. Preserve diagnostic artifacts and summarize the relevant spans, allocation sites, or syscall changes in the review evidence.

# Performance toolbox

Select tools based on the hypothesis; do not run every tool mechanically or treat instrumented timings as end-to-end evidence.

- Use Divan for statistically rigorous Rust microbenchmarks of hot paths after a representative workload identifies or motivates the path. Keep the end-to-end workload as the primary evidence.
- Use Loom to exhaustively explore thread interleavings when changing hand-rolled synchronization. Its model validates concurrency behavior, not production performance.
- Use Rust's ThreadSanitizer to detect data races in threaded code under representative tests or workloads. Record the nightly toolchain and sanitizer flags, and do not use sanitizer timings for performance claims.
- Use `cargo-bloat` for native Rust binary contribution analysis and `twiggy` for WebAssembly size analysis. Compare identical release profiles and report both absolute and relative size changes.
- Use `tokio-console` to inspect live async tasks, resources, wakeups, and stalls when the hypothesis involves the Turborepo daemon. Treat console output as diagnostic evidence and corroborate it with an end-to-end daemon workload.
- Use Linux `perf` for CPU profiles and hardware counters such as cache misses and branch mispredictions. Keep builds, workloads, and counter collection comparable, and use the counters to explain rather than replace uninstrumented timing results.

# Workload corpus

Use the local Turborepo checkout plus relevant repositories from this pinned corpus. Clone external repositories outside the checkout and detach at the listed commit. Run the locally built baseline and candidate binaries directly; do not benchmark each repository's declared `turbo` dependency. Start with non-mutating commands such as `turbo run <task> --dry=json --daemon=false` or `turbo ls`, then add real task execution only when the hypothesis concerns execution or caching.

- Small template: `https://github.com/t3-oss/create-t3-turbo.git` at `8f945b7bb3bfb3ca8358d48b1ff0214079bc11ee`.
- Small production monorepo: `https://github.com/dubinc/dub.git` at `1f30d5901e172b93f1c3ca8f24eebe778c6e4a75`.
- Medium production monorepo: `https://github.com/triggerdotdev/trigger.dev.git` at `442702e87902032d8b26d69c40a6a797994b91e2`.
- Large framework monorepo: `https://github.com/payloadcms/payload.git` at `24ac89558c7d778b6dc563f0ed82178d4b1c58bb`.
- Very large application monorepo: `https://github.com/calcom/cal.com.git` at `176037d0afbe572f870a3c702985e7cd83fe6c0c`.
- Large Bun monorepo: `https://github.com/onejs/one.git` at `9483d947c8fb88afb90e59c22da51d099047c426`.
- Large mixed Rust/JavaScript monorepo: the current `vercel/turborepo` checkout containing the candidate change.

The pinned external revisions contain roughly 12, 15, 50, 74, 81, and 119 checked-in `package.json` files, respectively. These counts are corpus-size indicators, not substitutes for recording the package and task graph exercised by a command.

Choose workloads before seeing results. For a general CLI, package-discovery, task-graph, hashing, or startup claim, test the local checkout and at least two external corpus entries of materially different measured graph sizes. Use more or all entries when claiming broad scaling improvements. Include the relevant pnpm, Yarn, or Bun entry for a package-manager or lockfile claim; construct and retain a pinned fixture when the affected case is not represented. A narrowly platform- or feature-specific change may use fewer repositories, but explain why the selected workload represents the affected population. Do not silently discard a repository where the candidate regresses or fails.

# Correctness

Run targeted tests, type checks, lint, or builds appropriate to every changed area. After any edit, rerun the after measurement and all correctness checks so the evidence fingerprints the final diff.

# Adversarial review

Use only the reviewer returned by `begin_performance_improvement`; it is the opposite model from the author. Give it the complete diff and all evidence. Require structured `approved`, `blockingFindings`, and `summary` fields. Resolve every blocking finding and obtain a new review after changes. Record only the actual returned verdict.

# Publishing

Open a draft PR only through `create_pull_request` after all gates pass. Use a `perf: Description` title with an uppercase description and no scope. The body must state the author and reviewer models, hypothesis, exact methodology, before/after evidence, validation, review outcome, and limitations.

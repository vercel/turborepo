---
description: Use when finding, measuring, implementing, reviewing, and publishing Turborepo performance improvements.
---

# Scope

Make one focused performance change that can be defended with repeatable measurements. Avoid dependency-only changes, broad refactors, generated files, release machinery, credentials, `.github/`, and this agent's own `apps/agents/` implementation.

# Measurement

1. Form a concrete performance hypothesis from code inspection.
2. Use a deterministic existing workload or add a narrowly scoped benchmark when needed.
3. Record the baseline before any checkout changes.
4. Change one relevant variable.
5. Run the exact baseline command again under comparable conditions.
6. Treat noisy, ambiguous, or non-reproducible results as no improvement.

Do not claim a percentage that the captured output does not support. Include relevant latency, throughput, allocation, memory, or build-time tradeoffs.

# Correctness

Run targeted tests, type checks, lint, or builds appropriate to every changed area. After any edit, rerun the after measurement and all correctness checks so the evidence fingerprints the final diff.

# Adversarial review

Use only the reviewer returned by `begin_performance_improvement`; it is the opposite model from the author. Give it the complete diff and all evidence. Require structured `approved`, `blockingFindings`, and `summary` fields. Resolve every blocking finding and obtain a new review after changes. Record only the actual returned verdict.

# Publishing

Open a draft PR only through `create_pull_request` after all gates pass. Use a `perf: Description` title with an uppercase description and no scope. The body must state the author and reviewer models, hypothesis, exact methodology, before/after evidence, validation, review outcome, and limitations.

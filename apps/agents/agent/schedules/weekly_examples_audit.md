---
cron: "0 14 * * 1"
---

Maintain every Turborepo example. Audit and update all stale dependencies, package-manager pins, Node engines, README instructions, versioned references, and inconsistent `turbo.json` tasks. Use exact latest stable versions, apply required best-practice migrations, regenerate lockfiles with each example's package manager, and run every relevant non-persistent validation task. Fix validation failures rather than stopping at an audit report.

When changes exist, call `create_pull_request` to open a draft pull request against `vercel/turborepo`. Summarize changed examples and validation results in the body. The tool supplies the scheduled branch, required Conventional Commit title, and commit message. If there are no changes, finish without creating a pull request.

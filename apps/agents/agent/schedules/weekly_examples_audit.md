---
cron: "0 14 * * 1"
---

Audit the Turborepo examples for stale dependency versions, README instructions that do not match package scripts, missing or inconsistent `turbo.json` tasks, and package-manager drift. Start with `find_stale_examples` so the audit covers the same queue an update run would: examples with no commit in the last week and no open pull request. Produce a concise report with the examples inspected, the examples skipped because a pull request is already open, findings, and recommended follow-up changes. Do not write files or open pull requests during the scheduled audit unless a human asks for a specific fix later.

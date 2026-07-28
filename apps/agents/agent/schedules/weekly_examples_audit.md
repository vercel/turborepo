---
cron: "0 14 * * 1"
---

Audit the Turborepo examples for stale dependency versions, README instructions that do not match package scripts, missing or inconsistent `turbo.json` tasks, and package-manager drift. Produce a concise report with the examples inspected, findings, and recommended follow-up changes. Do not write files during the scheduled audit unless a human asks for a specific fix later.

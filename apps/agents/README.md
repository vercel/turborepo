# Turborepo Agents

## Remote OpenCode maintenance

Manual daily-example maintenance can run in the shared OpenCode control plane. Performance runs and scheduled Eve automation are unchanged.

Set all of the following to enable it:

- `OPENCODE_SERVER_URL`: HTTPS origin of the shewbox master Sandbox proxy.
- `OPENCODE_SERVER_TOKEN`: bearer token matching shewbox's `OPENCODE_HARNESS_TOKEN`.
- `OPERATOR_RUN_SECRET`: random secret used to sign status URLs.

`OPENCODE_SERVER_PASSWORD` is supported instead of `OPENCODE_SERVER_TOKEN` for a direct OpenCode server, but not for shewbox. Configure exactly one authentication method. Incomplete configuration leaves manual maintenance on the existing Eve path.

The current slice accepts text prompts, fixes the workspace at `/workspace/projects/turborepo`, and executes tools inside OpenCode. It does not support Harness host tools or suspended-turn continuation. Inspect the shared OpenCode UI for the transcript, tool activity, and file changes.

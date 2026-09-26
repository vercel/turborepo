# Turborepo agent evals

Agent coding tasks live **in this repository**, beside the Turborepo code and docs they test. Each `evals/evals/<name>/` directory is a small npm-workspace monorepo with a user-facing `PROMPT.md`, a withheld Vitest grader in `EVAL.ts`, and a starter project. `@vercel/agent-eval` gives the agent only the prompt and starter project, then runs the grader against its work. Do not put the expected fix in the prompt or an agent-visible file.

## Run

From the repository root (Node 24, pnpm 12):

```sh
pnpm install --frozen-lockfile
pnpm eval <fixture> --dry                       # list work without model/sandbox calls
pnpm eval <fixture>                             # compare baseline and bundled-docs variants
pnpm eval <fixture> --variant agents-md --runs 4 # repeat to measure variability
pnpm eval --all                                # explicitly run every task
pnpm eval:check                               # local grader checks and typecheck; no model calls
```

For a live run, supply `AI_GATEWAY_API_KEY` (or Vercel OIDC credentials via `.env.local` under `evals/`) and either Docker or Vercel Sandbox. The default agent uses Claude Code through the AI Gateway and the model is `claude-sonnet-4-6`; set `EVAL_AGENT=claude-code` with `ANTHROPIC_API_KEY` for direct Anthropic access, and set `EVAL_MODEL` to an exact supported model ID to override the model. Results and transcripts are written under ignored `evals/results/`. Live runs consume provider credits; `--dry` and `eval:check` do not. `--dry` exits nonzero when the most recent result for a selected eval lacks grader output (for example, an AI Gateway 402); the framework's fingerprint status alone may incorrectly call such a run up to date. Fix the infrastructure problem and rerun that eval; do not count it as a model failure.

The runner packs `packages/turbo` on **every live run** (`prepack` bundles the current docs), installs that tarball in each sandbox, and checks that the docs and executable exist. Starter fixture manifests pin an older stable `turbo` for local grader checks; the live sandbox overlays it with the checkout's tarball. The matched native `@turbo/<platform>` binary comes from the **published checkout package version**, not a local Rust build. Do not use these evals to claim that unpublished Rust changes are being tested. Run the Rust integration suite for CLI changes; a future Linux-binary upload would be needed to eval an unpublished CLI build. If the native package version has not been published, the sandbox setup fails rather than silently using another version.

`baseline` and `agents-md` use the same fixture, model, and grader. Only the second writes `AGENTS.md` pointing to `node_modules/turbo/docs/README.md` (and compatibility links for other agents). The runner uses `--force` for live runs because changes to the packed package are not captured by the fixture fingerprint. Repeated runs use `earlyExit: false` so you can measure the actual pass rate; a single run is best for initial debugging.

To add an eval, create a directory under `evals/evals/` with an npm workspace fixture, `PROMPT.md`, and `EVAL.ts`. Prefer reproducible behavior tests (`turbo run --dry=json`, cache hashes, actual task outputs) to asserting one particular spelling in `turbo.json`. Keep the grader independent of the agent's file layout where possible. Add a red/green case in `tests/graders.test.mjs`: the starter should fail, and a known-correct solution should pass. Commit the fixture and its docs or CLI change together. CI can run `pnpm eval:check` without credentials; paid model runs are opt-in.

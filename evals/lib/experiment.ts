import type { ExperimentConfig } from "@vercel/agent-eval";
import { installTurbo, writeAgentsMd } from "./setup.js";

export function createExperiment(withDocs: boolean): ExperimentConfig {
  const runs = Number(process.env.EVAL_RUNS ?? "1");
  return {
    agent: process.env.EVAL_AGENT ?? "vercel-ai-gateway/claude-code",
    model: process.env.EVAL_MODEL ?? "claude-sonnet-4-6",
    evals: process.env.EVAL_FILTER ?? "*",
    scripts: ["build"],
    runs,
    earlyExit: runs === 1,
    timeout: 900,
    sandbox: "auto",
    setup: async (sandbox) => {
      await installTurbo(sandbox);
      if (withDocs) await writeAgentsMd(sandbox);
    }
  };
}

import { getVercelOidcToken } from "@vercel/oidc";
import { defineTool } from "eve/tools";
import { z } from "zod";

import { isAppPrincipal, resolveAutomatedExample } from "../lib/repo.js";

const checkout = "turborepo";
const model = "vercel/openai/gpt-5.6-sol";
const maxOutputLength = 20_000;
const timeout = "45m";

const inputSchema = z.object({
  example: z
    .string()
    .optional()
    .describe(
      "Example to maintain. Automated runs are restricted to today's selected example."
    )
});

export default defineTool({
  description:
    "Run GPT 5.6 Sol in the OpenCode harness to maintain one Turborepo example. The harness edits and validates the sandbox checkout but cannot publish changes.",
  inputSchema,
  approval: ({ session }) =>
    isAppPrincipal(session.auth.current) ? "not-applicable" : "user-approval",
  async execute(input, ctx) {
    const sandbox = await ctx.getSandbox();
    const example = await resolveAutomatedExample(
      sandbox,
      ctx.session.auth.current,
      ctx.session.id,
      input.example
    );
    if (!example) {
      throw new Error("Choose an example before running OpenCode maintenance.");
    }

    const prompt = `Maintain only the Turborepo example at examples/${example}. Audit and update its stale dependencies, package-manager pin, Node engine, README instructions, versioned references, and turbo.json tasks. Use exact latest stable versions, apply required best-practice migrations, regenerate its lockfile with its declared package manager, and run every relevant non-persistent validation task. Fix validation failures. Do not inspect or modify another example. Do not commit, push, or create a pull request. Finish with a concise summary of changes and validation results.`;
    const result = await sandbox.run({
      command: `timeout --signal=TERM ${timeout} opencode run --auto --format json --model ${model} --title ${shellQuote(`Maintain ${example} example`)} --dir . ${shellQuote(prompt)}`,
      workingDirectory: checkout,
      env: {
        AI_GATEWAY_API_KEY: await getVercelOidcToken(),
        CI: "1",
        OPENCODE_CONFIG_CONTENT: JSON.stringify({
          $schema: "https://opencode.ai/config.json",
          enabled_providers: ["vercel"],
          model,
          share: "disabled"
        })
      }
    });

    if (result.exitCode === 124) {
      throw new Error(`OpenCode maintenance timed out after ${timeout}.`);
    }
    if (result.exitCode !== 0) {
      throw new Error(`OpenCode maintenance failed: ${result.stderr}`);
    }

    return {
      example,
      harness: "opencode",
      model: "openai/gpt-5.6-sol",
      output: extractText(result.stdout)
    };
  }
});

function extractText(output: string): string {
  const text = output
    .split("\n")
    .flatMap((line) => {
      try {
        const event = JSON.parse(line) as {
          type?: string;
          part?: { text?: string };
        };
        return event.type === "text" && event.part?.text
          ? [event.part.text]
          : [];
      } catch {
        return [];
      }
    })
    .join("\n\n");
  const summary = text || "OpenCode completed without a text summary.";
  return summary.length <= maxOutputLength
    ? summary
    : `${summary.slice(0, maxOutputLength)}\n[output truncated]`;
}

function shellQuote(value: string): string {
  return `'${value.replaceAll("'", `'\\''`)}'`;
}

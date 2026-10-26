import { readFileSync } from "node:fs";
import type { EvalRunData, Sandbox } from "@vercel/agent-eval";
import ts from "typescript";

/** Packaged docs come from this checkout. The native binary comes from the
 * matching published @turbo/<platform> optional dependency in the sandbox. */
export async function installTurbo(sandbox: Sandbox): Promise<void> {
  const tarball = process.env.TURBO_EVAL_TARBALL;
  if (!tarball)
    throw new Error(
      "No turbo tarball. Run via `pnpm eval` at the repository root."
    );

  await sandbox.writeFiles({
    // @ts-expect-error The sandbox runtime accepts Buffers for binary files.
    "turbo.tgz": readFileSync(tarball)
  });
  const install = await sandbox.runCommand("npm", ["install", "./turbo.tgz"]);
  if (install.exitCode !== 0) {
    throw new Error(
      `Installing local turbo failed (${install.exitCode}): ${install.stderr}`
    );
  }
  const check = await sandbox.runCommand("node", [
    "-e",
    "const fs = require('fs'); if (!fs.existsSync('node_modules/turbo/docs/README.md')) process.exit(1)"
  ]);
  if (check.exitCode !== 0)
    throw new Error("Packed turbo is missing docs/README.md");
  const binary = await sandbox.runCommand("node_modules/.bin/turbo", [
    "--version"
  ]);
  if (binary.exitCode !== 0) {
    throw new Error(
      `Installed turbo binary cannot run (${binary.exitCode}): ${binary.stderr}`
    );
  }
  const manifest = JSON.parse(
    await sandbox.readFile("node_modules/turbo/package.json")
  );
  if (binary.stdout.trim() !== manifest.version) {
    throw new Error(
      `turbo package ${manifest.version} selected binary ${binary.stdout.trim()}`
    );
  }
}

export async function proveGuidanceIsolation(sandbox: Sandbox): Promise<void> {
  const proof = await sandbox.runCommand("node", [
    "-e",
    `
const { existsSync, readFileSync } = require('node:fs');
const { execFileSync } = require('node:child_process');
const config = JSON.parse(readFileSync('turbo.json', 'utf8'));
if (config.agentGuidance !== false) {
  throw new Error('fixture turbo.json must set "agentGuidance": false');
}
const paths = ['AGENTS.md', 'CLAUDE.md', 'GEMINI.md'];
const snapshot = () => paths.map(path => existsSync(path) ? readFileSync(path, 'utf8') : null);
const before = snapshot();
if (before.some(text => text?.includes('<!-- BEGIN:turborepo-agent-rules -->'))) {
  throw new Error('fixture already contains automatic Turbo guidance');
}
execFileSync('node_modules/.bin/turbo', ['ls'], {
  env: { ...process.env, CLAUDECODE: '1', TURBO_TELEMETRY_DISABLED: '1' },
  timeout: 30000,
  stdio: 'pipe'
});
if (JSON.stringify(snapshot()) !== JSON.stringify(before)) {
  throw new Error('Turbo changed agent instructions despite the guidance opt-out');
}
`
  ]);
  if (proof.exitCode !== 0) {
    throw new Error(
      `Guidance isolation proof failed (${proof.exitCode}): ${proof.stderr}`
    );
  }
}

export function invalidateGuidanceContamination(
  runData: EvalRunData
): EvalRunData {
  const reasons: string[] = [];
  for (const path of runData.deletedFiles ?? []) {
    if (path === "turbo.json" || path === "turbo.jsonc") {
      reasons.push(
        `${path} was deleted, removing the agentGuidance: false opt-out`
      );
    }
  }
  for (const [path, content] of Object.entries(runData.generatedFiles ?? {})) {
    const text = content.toString("utf8");
    if (path === "turbo.json" || path === "turbo.jsonc") {
      const parsed = ts.parseConfigFileTextToJson(path, text);
      const config: unknown = parsed.config;
      if (
        parsed.error ||
        typeof config !== "object" ||
        config === null ||
        !("agentGuidance" in config) ||
        config.agentGuidance !== false
      ) {
        reasons.push(
          `${path} must remain valid JSON/JSONC with agentGuidance: false`
        );
      }
    }
    if (
      /(^|\/)(AGENTS|CLAUDE|GEMINI)\.md$/.test(path) &&
      (text.includes("<!-- BEGIN:turborepo-agent-rules -->") ||
        text.includes("<!-- END:turborepo-agent-rules -->"))
    ) {
      reasons.push(`${path} contains automatic Turbo guidance`);
    }
  }
  if (reasons.length === 0) return runData;
  const invalidAttempt = `Guidance isolation invalidated: ${reasons.join("; ")}`;
  return {
    ...runData,
    result: {
      ...runData.result,
      status: "failed",
      error: [runData.result.error, invalidAttempt].filter(Boolean).join("\n"),
      metadata: { ...runData.result.metadata, invalidAttempt }
    }
  };
}

export async function writeAgentsMd(sandbox: Sandbox): Promise<void> {
  await sandbox.writeFiles({
    "AGENTS.md": `# Turborepo documentation

Before changing Turborepo tasks, caching, or workspace configuration, read
\`node_modules/turbo/docs/README.md\` and the smallest relevant pages it links to.
These docs match the installed \`turbo\` package; do not rely on remembered behavior.
`,
    "CLAUDE.md": "@AGENTS.md\n",
    "GEMINI.md": "@AGENTS.md\n"
  });
}

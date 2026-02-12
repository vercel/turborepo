import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { Sandbox } from "@vercel/sandbox";
import { githubToken } from "./env";
import { createPullRequest } from "./github";
import { createRun, updateRun, appendLogs } from "./runs";

const REPO_URL = "https://github.com/vercel/turborepo.git";
const AGENT_SCRIPT_PATH = resolve(process.cwd(), "sandbox/audit-fix-agent.mjs");
const RESULTS_PATH = "/vercel/sandbox/results.json";

// Batch log writes to avoid hammering Blob storage on every line.
// Flushes every 3 seconds or when 50 lines accumulate.
const LOG_FLUSH_INTERVAL_MS = 3_000;
const LOG_FLUSH_LINE_THRESHOLD = 50;

function createLogBuffer(runId: string) {
  let buffer: string[] = [];
  let timer: ReturnType<typeof setInterval> | null = null;

  async function flush() {
    if (buffer.length === 0) return;
    const chunk = buffer.join("");
    buffer = [];
    try {
      await appendLogs(runId, chunk);
    } catch {
      // Best-effort -- don't crash the pipeline over log persistence
    }
  }

  function start() {
    timer = setInterval(() => void flush(), LOG_FLUSH_INTERVAL_MS);
  }

  function push(line: string) {
    buffer.push(line);
    if (buffer.length >= LOG_FLUSH_LINE_THRESHOLD) {
      void flush();
    }
  }

  async function stop() {
    if (timer) clearInterval(timer);
    await flush();
  }

  return { start, push, stop };
}

async function installCargoAudit(sandbox: InstanceType<typeof Sandbox>) {
  await sandbox.runCommand({
    cmd: "dnf",
    args: ["install", "-y", "rust", "cargo", "gcc", "openssl-devel"],
    sudo: true
  });
  await sandbox.runCommand("cargo", ["install", "cargo-audit"]);
}

interface AuditVulnerability {
  name: string;
  severity: string;
  title: string;
  url: string;
  fixAvailable: string | boolean;
}

export interface AuditResults {
  cargo: AuditVulnerability[];
  pnpm: AuditVulnerability[];
  cargoRaw: string;
  pnpmRaw: string;
}

export interface AgentResults {
  success: boolean;
  summary: string;
  vulnerabilitiesFixed: number;
  vulnerabilitiesRemaining: number;
  manifestsUpdated: string[];
  sourceFilesUpdated: string[];
  testsPass: boolean;
  auditsClean: boolean;
}

export interface AuditFixResult {
  branch: string;
  diff: string;
  agentResults: AgentResults;
}

export interface FixPRResult {
  prUrl: string;
  prNumber: number;
}

export async function runSecurityAudit(runId?: string): Promise<AuditResults> {
  const sandbox = await Sandbox.create({
    runtime: "node22",
    timeout: 18_000_000 // 5 hours
  });

  if (runId) {
    await updateRun(runId, {
      status: "scanning",
      sandboxId: (sandbox as unknown as { sandboxId?: string }).sandboxId
    });
  }

  try {
    await installCargoAudit(sandbox);
    await sandbox.runCommand("npm", ["install", "-g", "pnpm@10"]);
    await sandbox.runCommand("git", [
      "clone",
      "--depth",
      "1",
      REPO_URL,
      "turborepo"
    ]);

    const cargoResult = await sandbox.runCommand("bash", [
      "-c",
      "cd turborepo && cargo-audit audit --json 2>&1 || true"
    ]);
    const cargoRaw = await cargoResult.stdout();

    const pnpmResult = await sandbox.runCommand("bash", [
      "-c",
      "cd turborepo && pnpm audit --json 2>&1 || true"
    ]);
    const pnpmRaw = await pnpmResult.stdout();

    return {
      cargo: parseCargoAudit(cargoRaw),
      pnpm: parsePnpmAudit(pnpmRaw),
      cargoRaw,
      pnpmRaw
    };
  } finally {
    await sandbox.stop();
  }
}

export async function runAuditFix(
  onProgress?: (message: string) => Promise<void>,
  runId?: string
): Promise<AuditFixResult> {
  const token = githubToken();
  const aiGatewayKey = process.env.AI_GATEWAY_API_KEY;
  if (!aiGatewayKey) {
    throw new Error("AI_GATEWAY_API_KEY is required for the audit fix agent");
  }

  const branch = `fix/security-audit-${Date.now()}`;
  const sandbox = await Sandbox.create({
    runtime: "node22",
    resources: { vcpus: 4 },
    timeout: 18_000_000 // 5 hours
  });

  if (runId) {
    await updateRun(runId, {
      status: "fixing",
      sandboxId: (sandbox as unknown as { sandboxId?: string }).sandboxId,
      branch
    });
  }

  const logBuffer = runId ? createLogBuffer(runId) : null;
  logBuffer?.start();

  try {
    const log = (msg: string) => {
      logBuffer?.push(`[${new Date().toISOString()}] ${msg}\n`);
    };

    log("Installing tooling...");
    await onProgress?.("Installing tooling...");
    await installCargoAudit(sandbox);
    await sandbox.runCommand("npm", ["install", "-g", "pnpm@10"]);

    log("Cloning repository...");
    await onProgress?.("Cloning repository...");
    await sandbox.runCommand("bash", [
      "-c",
      `git clone --depth 1 https://x-access-token:${token}@github.com/vercel/turborepo.git turborepo`
    ]);

    await sandbox.runCommand("bash", [
      "-c",
      [
        "cd turborepo",
        'git config user.name "turborepo-agents[bot]"',
        'git config user.email "turborepo-agents[bot]@users.noreply.github.com"',
        `git checkout -b ${branch}`
      ].join(" && ")
    ]);

    log("Installing agent dependencies...");
    await onProgress?.("Installing agent dependencies...");
    await sandbox.runCommand("npm", ["install", "ai", "zod"]);

    const agentScript = readFileSync(AGENT_SCRIPT_PATH);
    await sandbox.writeFiles([
      { path: "/vercel/sandbox/audit-fix-agent.mjs", content: agentScript }
    ]);

    log("Running audit fix agent...");
    await onProgress?.("Running audit fix agent...");

    // Run the agent in detached mode so we can stream logs in real-time
    const agentCmd = await sandbox.runCommand({
      cmd: "bash",
      args: [
        "-c",
        `AI_GATEWAY_API_KEY=${aiGatewayKey} node audit-fix-agent.mjs`
      ],
      detached: true
    });

    // Stream sandbox stdout/stderr into the log buffer
    for await (const entry of agentCmd.logs()) {
      const prefix = entry.stream === "stderr" ? "[stderr] " : "";
      log(`${prefix}${entry.data}`);
    }

    const agentResult = await agentCmd.wait();

    if (agentResult.exitCode !== 0) {
      const stdout = await agentResult.stdout();
      const stderr = await agentResult.stderr();
      log(`Agent exited with code ${agentResult.exitCode}`);
      log(`stdout (last 2000): ${stdout.slice(-2000)}`);
      log(`stderr (last 2000): ${stderr.slice(-2000)}`);
      console.error("Agent stdout:", stdout.slice(-2000));
      console.error("Agent stderr:", stderr.slice(-2000));
      throw new Error(
        `Agent exited with code ${agentResult.exitCode}: ${stderr.slice(0, 500)}`
      );
    }

    const resultsBuffer = await sandbox.readFileToBuffer({
      path: RESULTS_PATH
    });
    if (!resultsBuffer) {
      throw new Error("Agent did not produce a results file");
    }
    const agentResults: AgentResults = JSON.parse(
      resultsBuffer.toString("utf-8")
    );
    log(`Agent results: ${JSON.stringify(agentResults)}`);

    if (agentResults.manifestsUpdated.length === 0) {
      throw new Error(
        `Agent completed but no manifests were updated. Summary: ${agentResults.summary}`
      );
    }

    const diffResult = await sandbox.runCommand("bash", [
      "-c",
      "cd turborepo && git diff && git diff --cached"
    ]);
    const diff = await diffResult.stdout();

    log("Pushing branch...");
    await onProgress?.("Pushing branch...");
    if (runId) {
      await updateRun(runId, { status: "pushing" });
    }

    const pushResult = await sandbox.runCommand("bash", [
      "-c",
      [
        "cd turborepo",
        "git add -A",
        'git commit -m "fix: Patch vulnerable dependencies"',
        `git push origin ${branch}`
      ].join(" && ")
    ]);

    if (pushResult.exitCode !== 0) {
      const stderr = await pushResult.stderr();
      throw new Error(`git push failed: ${stderr}`);
    }

    log("Done.");
    return { branch, diff, agentResults };
  } finally {
    await logBuffer?.stop();
    await sandbox.stop();
  }
}

export async function openFixPR(
  branch: string,
  agentResults: AgentResults
): Promise<FixPRResult> {
  const bodyLines = [
    "This PR was automatically generated by the Turborepo security audit agent.",
    "",
    `**${agentResults.vulnerabilitiesFixed}** vulnerabilities fixed, **${agentResults.vulnerabilitiesRemaining}** remaining.`,
    `Tests passing: ${agentResults.testsPass ? "yes" : "no"}`,
    `Audits clean: ${agentResults.auditsClean ? "yes" : "no"}`,
    "",
    "## Summary",
    "",
    agentResults.summary,
    ""
  ];

  if (agentResults.manifestsUpdated.length > 0) {
    bodyLines.push("## Updated manifests", "");
    for (const f of agentResults.manifestsUpdated) {
      bodyLines.push(`- \`${f}\``);
    }
    bodyLines.push("");
  }

  if (agentResults.sourceFilesUpdated.length > 0) {
    bodyLines.push("## Source code changes", "");
    for (const f of agentResults.sourceFilesUpdated) {
      bodyLines.push(`- \`${f}\``);
    }
    bodyLines.push("");
  }

  bodyLines.push(
    "---",
    "Review the changes carefully. CI must pass before merging."
  );

  const pr = await createPullRequest({
    title: "fix: Patch vulnerable dependencies",
    body: bodyLines.join("\n"),
    head: branch
  });

  return { prUrl: pr.html_url, prNumber: pr.number };
}

// The full audit-and-fix pipeline: scan, run agent, post results to Slack.
export async function runAuditAndFix(
  trigger: "cron" | "manual" = "manual"
): Promise<void> {
  const { slackChannel } = await import("./env");
  const { postMessage, updateMessage } = await import("./slack");

  const run = await createRun(trigger);

  let results: AuditResults;
  try {
    results = await runSecurityAudit(run.id);
  } catch (error) {
    console.error("Audit failed:", error);
    const msg = error instanceof Error ? error.message : String(error);
    await updateRun(run.id, { status: "failed", error: msg });
    await postMessage(
      slackChannel(),
      ":x: Security audit failed to run. Check the logs."
    );
    return;
  }

  const totalVulns = results.cargo.length + results.pnpm.length;
  await updateRun(run.id, {
    vulnerabilities: { cargo: results.cargo.length, pnpm: results.pnpm.length }
  });

  if (totalVulns === 0) {
    await updateRun(run.id, { status: "completed" });
    await postMessage(
      slackChannel(),
      ":white_check_mark: Security audit passed. 0 vulnerabilities found in cargo and pnpm."
    );
    return;
  }

  const header = `:wrench: *Security audit: fixing ${totalVulns} vulnerabilities*`;

  const statusMsg = await postMessage(
    slackChannel(),
    `${header}\n:hourglass_flowing_sand: Starting fix agent...`
  );

  const channel = slackChannel();
  const ts = statusMsg.ts as string;

  const onProgress = async (message: string) => {
    await updateMessage(
      channel,
      ts,
      `${header}\n:hourglass_flowing_sand: ${message}`
    );
  };

  try {
    const fixResult = await runAuditFix(onProgress, run.id);
    const { agentResults: r, branch, diff } = fixResult;

    const { uploadDiff } = await import("./blob");
    const diffUrl = await uploadDiff(diff, branch);

    await updateRun(run.id, {
      status: "completed",
      branch,
      diffUrl,
      agentResults: {
        success: r.success,
        summary: r.summary,
        vulnerabilitiesFixed: r.vulnerabilitiesFixed,
        vulnerabilitiesRemaining: r.vulnerabilitiesRemaining
      }
    });

    const appUrl = process.env.VERCEL_PROJECT_PRODUCTION_URL
      ? `https://${process.env.VERCEL_PROJECT_PRODUCTION_URL}`
      : process.env.VERCEL_URL
        ? `https://${process.env.VERCEL_URL}`
        : "http://localhost:3000";
    const viewUrl = `${appUrl}/vuln-diffs/view?url=${encodeURIComponent(diffUrl)}`;

    const statusLine = [
      `${r.vulnerabilitiesFixed} fixed, ${r.vulnerabilitiesRemaining} remaining`,
      `tests: ${r.testsPass ? "passing" : "failing"}`,
      `audits: ${r.auditsClean ? "clean" : "not clean"}`
    ].join(" · ");

    await updateMessage(
      channel,
      ts,
      `Audit fix ready for review (branch: ${branch})`,
      [
        {
          type: "section" as const,
          text: {
            type: "mrkdwn" as const,
            text: `:white_check_mark: *Audit fix agent finished*\n${statusLine}`
          }
        },
        {
          type: "section" as const,
          text: {
            type: "mrkdwn" as const,
            text: `*Summary:* ${r.summary}`
          }
        },
        {
          type: "section" as const,
          text: {
            type: "mrkdwn" as const,
            text: `<${viewUrl}|View diff> · <${diffUrl}|Download .patch>`
          }
        },
        {
          type: "actions" as const,
          elements: [
            {
              type: "button" as const,
              text: { type: "plain_text" as const, text: "Open PR" },
              style: "primary" as const,
              action_id: "audit_open_pr",
              value: JSON.stringify({ branch, agentResults: r })
            },
            {
              type: "button" as const,
              text: { type: "plain_text" as const, text: "Dismiss" },
              action_id: "audit_dismiss"
            }
          ]
        }
      ]
    );
  } catch (error) {
    console.error("Audit fix agent failed:", error);
    const msg = error instanceof Error ? error.message : String(error);
    await updateRun(run.id, { status: "failed", error: msg });
    await updateMessage(channel, ts, `:x: Audit fix agent failed: ${msg}`);
  }
}

function parseCargoAudit(raw: string): AuditVulnerability[] {
  try {
    const data = JSON.parse(raw);
    const vulnerabilities = data.vulnerabilities?.list ?? [];
    return vulnerabilities.map(
      (v: {
        advisory: {
          id: string;
          package: string;
          title: string;
          url: string;
          cvss?: string;
        };
      }) => ({
        name: `${v.advisory.package} (${v.advisory.id})`,
        severity: v.advisory.cvss ?? "unknown",
        title: v.advisory.title,
        url: v.advisory.url,
        fixAvailable: "check advisory"
      })
    );
  } catch {
    return [];
  }
}

function parsePnpmAudit(raw: string): AuditVulnerability[] {
  try {
    const data = JSON.parse(raw);
    const advisories = data.advisories ?? {};
    return (
      Object.values(advisories) as Array<{
        module_name?: string;
        severity?: string;
        title?: string;
        url?: string;
        fixAvailable?: boolean;
      }>
    ).map((a) => ({
      name: a.module_name ?? "unknown",
      severity: a.severity ?? "unknown",
      title: a.title ?? "",
      url: a.url ?? "",
      fixAvailable: a.fixAvailable ?? false
    }));
  } catch {
    return [];
  }
}

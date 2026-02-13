import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { Writable } from "node:stream";
import { Sandbox } from "@vercel/sandbox";
import { uploadDiff } from "./blob";
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
  let flushInProgress: Promise<void> | null = null;

  async function flush() {
    // Wait for any in-flight flush to finish so we don't have two
    // concurrent read-modify-write cycles on the same blob.
    if (flushInProgress) await flushInProgress;

    if (buffer.length === 0) return;

    // Swap the buffer synchronously so new pushes go to a fresh array
    // while we're writing. This prevents data loss.
    const chunk = buffer.join("");
    buffer = [];

    const work = appendLogs(runId, chunk).catch(() => {
      // Best-effort -- don't crash the pipeline over log persistence
    });
    flushInProgress = work;
    await work;
    flushInProgress = null;
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

function createLogWritable(log: (msg: string) => void): Writable {
  return new Writable({
    write(chunk, _encoding, callback) {
      log(chunk.toString());
      callback();
    }
  });
}

async function installCargoAudit(
  sandbox: InstanceType<typeof Sandbox>,
  outputStreams?: { stdout: Writable; stderr: Writable }
) {
  await sandbox.runCommand({
    cmd: "dnf",
    args: ["install", "-y", "rust", "cargo", "gcc", "openssl-devel"],
    sudo: true,
    ...outputStreams
  });
  await sandbox.runCommand({
    cmd: "cargo",
    args: ["install", "cargo-audit"],
    ...outputStreams
  });
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
  patchUrl: string;
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
  const aiGatewayKey = process.env.AI_GATEWAY_API_KEY;
  if (!aiGatewayKey) {
    throw new Error(
      "AI_GATEWAY_API_KEY is required for the security audit fix agent"
    );
  }

  const sandbox = await Sandbox.create({
    runtime: "node22",
    resources: { vcpus: 4 },
    timeout: 18_000_000 // 5 hours
  });

  if (runId) {
    await updateRun(runId, {
      status: "fixing",
      sandboxId: (sandbox as unknown as { sandboxId?: string }).sandboxId
    });
  }

  const logBuffer = runId ? createLogBuffer(runId) : null;
  logBuffer?.start();

  try {
    const log = (msg: string) => {
      logBuffer?.push(`[${new Date().toISOString()}] ${msg}\n`);
    };

    const outputStreams = logBuffer
      ? {
          stdout: createLogWritable(log),
          stderr: createLogWritable((msg) => log(`[stderr] ${msg}`))
        }
      : undefined;

    log("Installing tooling...");
    await onProgress?.("Installing tooling...");
    await installCargoAudit(sandbox, outputStreams);
    await sandbox.runCommand({
      cmd: "npm",
      args: ["install", "-g", "pnpm@10"],
      ...outputStreams
    });

    log("Cloning repository...");
    await onProgress?.("Cloning repository...");
    await sandbox.runCommand({
      cmd: "git",
      args: ["clone", "--depth", "1", REPO_URL, "turborepo"],
      ...outputStreams
    });

    log("Installing agent dependencies...");
    await onProgress?.("Installing agent dependencies...");
    await sandbox.runCommand({
      cmd: "npm",
      args: ["install", "ai", "zod"],
      ...outputStreams
    });

    const agentScript = readFileSync(AGENT_SCRIPT_PATH);
    await sandbox.writeFiles([
      { path: "/vercel/sandbox/audit-fix-agent.mjs", content: agentScript }
    ]);

    log("Running security audit fixer agent...");
    await onProgress?.("Running security audit fixer agent...");

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

    // Generate a patch from uncommitted changes and upload to Blob
    log("Generating patch...");
    await onProgress?.("Generating patch...");
    const patchResult = await sandbox.runCommand("bash", [
      "-c",
      "cd turborepo && git add -A && git diff --cached"
    ]);
    const patch = await patchResult.stdout();

    if (!patch.trim()) {
      throw new Error(
        "Agent reported changes but git diff produced an empty patch"
      );
    }

    const patchUrl = await uploadDiff(patch, `security-audit-${Date.now()}`);
    log(`Patch uploaded: ${patchUrl}`);

    log("Done.");
    return { patchUrl, agentResults };
  } finally {
    await logBuffer?.stop();
    await sandbox.stop();
  }
}

export async function openFixPR(
  patchUrl: string,
  agentResults: AgentResults
): Promise<FixPRResult> {
  const token = githubToken();
  const branch = `fix/security-audit-${Date.now()}`;

  const sandbox = await Sandbox.create({
    runtime: "node22",
    timeout: 600_000 // 10 minutes is plenty for clone + apply + push
  });

  try {
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

    // Download the patch from Blob and apply it
    const { get } = await import("@vercel/blob");
    const blob = await get(patchUrl, { access: "private" });
    if (!blob) throw new Error("Patch blob not found");
    const patch = await new Response(blob.stream).text();

    await sandbox.writeFiles([
      { path: "/vercel/sandbox/fix.patch", content: Buffer.from(patch) }
    ]);

    const applyResult = await sandbox.runCommand("bash", [
      "-c",
      "cd turborepo && git apply /vercel/sandbox/fix.patch"
    ]);
    if (applyResult.exitCode !== 0) {
      const stderr = await applyResult.stderr();
      throw new Error(`git apply failed: ${stderr}`);
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
  } finally {
    await sandbox.stop();
  }

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

function dashboardUrl(runId: string): string {
  const appUrl = process.env.VERCEL_PROJECT_PRODUCTION_URL
    ? `https://${process.env.VERCEL_PROJECT_PRODUCTION_URL}`
    : process.env.VERCEL_URL
      ? `https://${process.env.VERCEL_URL}`
      : "http://localhost:3000";
  return `${appUrl}/dashboard/${runId}`;
}

// The full audit-and-fix pipeline: scan, run agent, post results to Slack.
export async function runAuditAndFix(
  trigger: "cron" | "manual" = "manual"
): Promise<void> {
  const { slackChannel } = await import("./env");
  const { postMessage, updateMessage } = await import("./slack");

  const run = await createRun(trigger);
  const runUrl = dashboardUrl(run.id);

  const statusMsg = await postMessage(
    slackChannel(),
    `:hourglass_flowing_sand: Security audit started — <${runUrl}|view run>`
  );

  const channel = slackChannel();
  const ts = statusMsg.ts as string;

  let results: AuditResults;
  try {
    results = await runSecurityAudit(run.id);
  } catch (error) {
    console.error("Audit failed:", error);
    const msg = error instanceof Error ? error.message : String(error);
    await updateRun(run.id, { status: "failed", error: msg });
    await updateMessage(
      channel,
      ts,
      `:x: Security audit failed — <${runUrl}|view run>`
    );
    return;
  }

  const totalVulns = results.cargo.length + results.pnpm.length;
  await updateRun(run.id, {
    vulnerabilities: { cargo: results.cargo.length, pnpm: results.pnpm.length }
  });

  if (totalVulns === 0) {
    await updateRun(run.id, { status: "completed" });
    await updateMessage(
      channel,
      ts,
      `:white_check_mark: Security audit clean — 0 vulnerabilities — <${runUrl}|view run>`
    );
    return;
  }

  const onProgress = async (message: string) => {
    await updateMessage(
      channel,
      ts,
      `:hourglass_flowing_sand: ${message} (${totalVulns} vulns) — <${runUrl}|view run>`
    );
  };

  try {
    const fixResult = await runAuditFix(onProgress, run.id);
    const { agentResults: r, patchUrl } = fixResult;

    await updateRun(run.id, {
      status: "completed",
      diffUrl: patchUrl,
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
    const viewDiffUrl = `${appUrl}/vuln-diffs/view?url=${encodeURIComponent(patchUrl)}`;

    await updateMessage(
      channel,
      ts,
      `:white_check_mark: Security audit fixes complete — ${r.vulnerabilitiesFixed} fixed, ${r.vulnerabilitiesRemaining} remaining — <${viewDiffUrl}|View patch> · <${runUrl}|View run>`
    );
  } catch (error) {
    console.error("Security audit fix agent failed:", error);
    const msg = error instanceof Error ? error.message : String(error);
    await updateRun(run.id, { status: "failed", error: msg });
    await updateMessage(
      channel,
      ts,
      `:x: Security audit fix failed — <${runUrl}|view run>`
    );
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

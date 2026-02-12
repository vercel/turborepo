import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { Sandbox } from "@vercel/sandbox";
import { uploadDiff } from "./blob";
import { createRun, updateRun, appendLogs, type RunMeta } from "./runs";

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

  return { start, push, flush, stop };
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
  diff: string;
  agentResults: AgentResults;
}

async function setupSandbox(
  sandbox: InstanceType<typeof Sandbox>,
  log: (msg: string) => void,
  logBuffer: ReturnType<typeof createLogBuffer> | null
): Promise<void> {
  log("Installing system packages (rust, cargo, gcc, openssl-devel)...");
  await logBuffer?.flush();

  const dnf = await sandbox.runCommand({
    cmd: "dnf",
    args: ["install", "-y", "rust", "cargo", "gcc", "openssl-devel"],
    sudo: true,
    detached: true
  });
  const dnfResult = await dnf.wait();
  if (dnfResult.exitCode !== 0) {
    const stderr = await dnfResult.stderr();
    throw new Error(
      `dnf install failed (exit ${dnfResult.exitCode}): ${stderr.slice(0, 500)}`
    );
  }

  log("Installing cargo-audit...");
  await logBuffer?.flush();

  const cargoInstall = await sandbox.runCommand({
    cmd: "cargo",
    args: ["install", "cargo-audit"],
    detached: true
  });
  const cargoResult = await cargoInstall.wait();
  if (cargoResult.exitCode !== 0) {
    const stderr = await cargoResult.stderr();
    throw new Error(
      `cargo install failed (exit ${cargoResult.exitCode}): ${stderr.slice(0, 500)}`
    );
  }

  log("Installing pnpm...");
  await logBuffer?.flush();
  await sandbox.runCommand("npm", ["install", "-g", "pnpm@10"]);

  log("Cloning repository...");
  await logBuffer?.flush();
  await sandbox.runCommand("git", [
    "clone",
    "--depth",
    "1",
    REPO_URL,
    "turborepo"
  ]);
}

async function scanAudits(
  sandbox: InstanceType<typeof Sandbox>,
  log: (msg: string) => void,
  logBuffer: ReturnType<typeof createLogBuffer> | null
): Promise<AuditResults> {
  log("Running cargo audit...");
  await logBuffer?.flush();
  const cargoResult = await sandbox.runCommand("bash", [
    "-c",
    "cd turborepo && cargo-audit audit --json 2>&1 || true"
  ]);
  const cargoRaw = await cargoResult.stdout();

  log("Running pnpm audit...");
  await logBuffer?.flush();
  const pnpmResult = await sandbox.runCommand("bash", [
    "-c",
    "cd turborepo && pnpm audit --json 2>&1 || true"
  ]);
  const pnpmRaw = await pnpmResult.stdout();

  const cargo = parseCargoAudit(cargoRaw);
  const pnpm = parsePnpmAudit(pnpmRaw);
  log(`Scan complete: ${cargo.length} cargo vulns, ${pnpm.length} pnpm vulns`);

  return { cargo, pnpm, cargoRaw, pnpmRaw };
}

export async function runSecurityAudit(runId?: string): Promise<AuditResults> {
  const sandbox = await Sandbox.create({
    runtime: "node22",
    timeout: 18_000_000 // 5 hours
  });
  const logBuffer = runId ? createLogBuffer(runId) : null;
  logBuffer?.start();

  const log = (msg: string) => {
    logBuffer?.push(`[${new Date().toISOString()}] ${msg}\n`);
  };

  if (runId) {
    await updateRun(runId, {
      status: "scanning",
      sandboxId: (sandbox as unknown as { sandboxId?: string }).sandboxId
    });
  }

  try {
    await setupSandbox(sandbox, log, logBuffer);
    return await scanAudits(sandbox, log, logBuffer);
  } finally {
    await logBuffer?.stop();
    await sandbox.stop();
  }
}

export async function runAuditFix(
  onProgress?: (message: string) => Promise<void>,
  runId?: string,
  existingSandbox?: InstanceType<typeof Sandbox>
): Promise<AuditFixResult> {
  const aiGatewayKey = process.env.AI_GATEWAY_API_KEY;
  if (!aiGatewayKey) {
    throw new Error("AI_GATEWAY_API_KEY is required for the audit fix agent");
  }

  const ownsBox = !existingSandbox;
  const sandbox =
    existingSandbox ??
    (await Sandbox.create({
      runtime: "node22",
      resources: { vcpus: 4 },
      timeout: 18_000_000 // 5 hours
    }));

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

    if (ownsBox) {
      log("Installing tooling...");
      await logBuffer?.flush();
      await onProgress?.("Installing tooling...");
      await setupSandbox(sandbox, log, logBuffer);
    }

    log("Installing agent dependencies...");
    await logBuffer?.flush();
    await onProgress?.("Installing agent dependencies...");
    await sandbox.runCommand("npm", ["install", "ai", "zod"]);

    const agentScript = readFileSync(AGENT_SCRIPT_PATH);
    await sandbox.writeFiles([
      { path: "/vercel/sandbox/audit-fix-agent.mjs", content: agentScript }
    ]);

    log("Running audit fix agent...");
    await logBuffer?.flush();
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

    // Generate a patch from the agent's changes and upload to Blob
    log("Generating patch...");
    await logBuffer?.flush();
    await onProgress?.("Generating patch...");

    await sandbox.runCommand("bash", ["-c", "cd turborepo && git add -A"]);
    const diffResult = await sandbox.runCommand("bash", [
      "-c",
      "cd turborepo && git diff --cached"
    ]);
    const diff = await diffResult.stdout();

    log("Uploading patch to Blob...");
    await logBuffer?.flush();
    const patchUrl = await uploadDiff(diff, `security-audit-${Date.now()}`);

    log("Done.");
    return { patchUrl, diff, agentResults };
  } finally {
    await logBuffer?.stop();
    if (ownsBox) {
      await sandbox.stop();
    }
  }
}

// The full audit-and-fix pipeline: scan, run agent, upload patch to Blob.
// Uses a single sandbox for both phases to avoid reinstalling tooling twice.
export async function runAuditAndFix(
  trigger: "cron" | "manual" = "manual"
): Promise<void> {
  const { slackChannel } = await import("./env");
  const { postMessage, updateMessage, replyInThread } = await import("./slack");

  const run = await createRun(trigger);

  const sandbox = await Sandbox.create({
    runtime: "node22",
    resources: { vcpus: 4 },
    timeout: 18_000_000 // 5 hours
  });

  const logBuffer = createLogBuffer(run.id);
  logBuffer.start();

  const log = (msg: string) => {
    logBuffer.push(`[${new Date().toISOString()}] ${msg}\n`);
  };

  let results: AuditResults;
  try {
    await updateRun(run.id, {
      status: "scanning",
      sandboxId: (sandbox as unknown as { sandboxId?: string }).sandboxId
    });

    await setupSandbox(sandbox, log, logBuffer);
    results = await scanAudits(sandbox, log, logBuffer);
  } catch (error) {
    console.error("Audit failed:", error);
    const msg = error instanceof Error ? error.message : String(error);
    await updateRun(run.id, { status: "failed", error: msg });
    await logBuffer.stop();
    await sandbox.stop();
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
    await logBuffer.stop();
    await sandbox.stop();
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
    const fixResult = await runAuditFix(onProgress, run.id, sandbox);
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
        : null;

    const parts: string[] = [];
    if (results.pnpm.length > 0) parts.push(`${results.pnpm.length} JS`);
    if (results.cargo.length > 0) parts.push(`${results.cargo.length} Rust`);
    const headline = `:white_check_mark: Audit fix: ${r.vulnerabilitiesFixed} of ${parts.join(" + ")} vulnerabilities patched`;

    await updateMessage(channel, ts, headline, [
      {
        type: "section" as const,
        text: { type: "mrkdwn" as const, text: headline }
      },
      ...(appUrl
        ? [
            {
              type: "actions" as const,
              elements: [
                {
                  type: "button" as const,
                  text: {
                    type: "plain_text" as const,
                    text: "View patch & copy apply command"
                  },
                  style: "primary" as const,
                  url: `${appUrl}/vuln-diffs/view?pathname=${encodeURIComponent(patchUrl)}`,
                  action_id: "audit_view_patch"
                },
                {
                  type: "button" as const,
                  text: { type: "plain_text" as const, text: "Dismiss" },
                  action_id: "audit_dismiss"
                }
              ]
            }
          ]
        : [])
    ]);

    await replyInThread(channel, ts, r.summary);
  } catch (error) {
    console.error("Audit fix agent failed:", error);
    const msg = error instanceof Error ? error.message : String(error);
    await updateRun(run.id, { status: "failed", error: msg });
    await updateMessage(channel, ts, `:x: Audit fix agent failed: ${msg}`);
  } finally {
    await logBuffer.stop();
    await sandbox.stop();
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

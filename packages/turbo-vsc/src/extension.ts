import {
  ExtensionContext,
  window,
  commands,
  workspace,
  StatusBarAlignment,
  StatusBarItem,
  Uri,
  env
} from "vscode";
import * as cp from "node:child_process";
import * as path from "node:path";
import * as fs from "node:fs";

import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions
} from "vscode-languageclient/node";

let client: LanguageClient;

let toolbar: StatusBarItem;

const logs = window.createOutputChannel("Turborepo Extension");

type InternalLspProbeResult =
  | { supported: true }
  | { supported: false; reason: string };

export function activate(context: ExtensionContext) {
  const workspaceRoot = workspace.workspaceFolders?.[0]?.uri.fsPath;
  const options = { cwd: workspaceRoot };
  const syncOptions: cp.ExecSyncOptionsWithStringEncoding = {
    ...options,
    encoding: "utf8"
  };

  const turboSettings = workspace.getConfiguration("turbo");
  const configuredTurboPath: string | undefined = turboSettings.get("path");
  const useLocalTurbo: boolean = turboSettings.get("useLocalTurbo") ?? false;
  let turboPath = resolveTurboPath(configuredTurboPath, workspaceRoot);

  logs.appendLine("starting the turbo extension");

  if (turboPath) {
    logs.appendLine(`using turbo at path ${turboPath}`);
  }

  const packagedLspPath = Uri.joinPath(
    context.extensionUri,
    "out",
    `turborepo-lsp-${process.platform}-${process.arch}${
      process.platform === "win32" ? ".exe" : ""
    }`
  ).fsPath;

  const installedTurboLspPath = findInstalledTurboLsp(
    workspaceRoot,
    syncOptions,
    turboPath
  );

  const daemonCommandPath = installedTurboLspPath ?? packagedLspPath;

  const getTurboPath = async () => {
    turboPath ??= findTurbo(workspaceRoot, syncOptions);
    if (turboPath) {
      return turboPath;
    }

    await promptGlobalTurbo(useLocalTurbo);
    turboPath = findTurbo(workspaceRoot, syncOptions);
    return turboPath;
  };

  const getDaemonCommandPath = async () => {
    if (fs.existsSync(daemonCommandPath)) {
      return daemonCommandPath;
    }

    return getTurboPath();
  };

  context.subscriptions.push(
    commands.registerCommand("turbo.daemon.start", async () => {
      const daemonPath = await getDaemonCommandPath();
      if (!daemonPath) {
        return;
      }

      cp.exec(`${quoteCommand(daemonPath)} daemon start`, options, (err) => {
        if (err) {
          if (err.message.includes("command not found")) {
            promptGlobalTurbo(useLocalTurbo);
          } else {
            logs.appendLine(`unable to start turbo: ${err.message}`);
          }
        } else {
          updateStatusBarItem(true);
          window.showInformationMessage("Turbo daemon started");
        }
      });
    })
  );

  context.subscriptions.push(
    commands.registerCommand("turbo.daemon.stop", async () => {
      const daemonPath = await getDaemonCommandPath();
      if (!daemonPath) {
        return;
      }

      cp.exec(`${quoteCommand(daemonPath)} daemon stop`, options, (err) => {
        if (err) {
          if (err.message.includes("command not found")) {
            promptGlobalTurbo(useLocalTurbo);
          } else {
            logs.appendLine(`unable to stop turbo: ${err.message}`);
          }
        } else {
          updateStatusBarItem(false);
          window.showInformationMessage("Turbo daemon stopped");
        }
      });
    })
  );

  context.subscriptions.push(
    commands.registerCommand("turbo.daemon.status", async () => {
      const daemonPath = await getDaemonCommandPath();
      if (!daemonPath) {
        return;
      }

      cp.exec(`${quoteCommand(daemonPath)} daemon status`, options, (err) => {
        if (err) {
          if (err.message.includes("command not found")) {
            promptGlobalTurbo(useLocalTurbo);
          } else {
            logs.appendLine(`unable to get turbo status: ${err.message}`);
            updateStatusBarItem(false);
          }
        } else {
          updateStatusBarItem(true);
          window.showInformationMessage("Turbo daemon is running");
        }
      });
    })
  );

  context.subscriptions.push(
    commands.registerCommand("turbo.run", async (args) => {
      const turboPath = await getTurboPath();
      if (!turboPath) {
        return;
      }

      const terminal = window.createTerminal({
        name: `${args}`,
        isTransient: true,
        iconPath: Uri.joinPath(context.extensionUri, "resources", "icon.svg")
      });
      terminal.sendText(`${quoteCommand(turboPath)} run ${args}`);
      terminal.show();
    })
  );

  context.subscriptions.push(
    commands.registerCommand("turbo.codemod", (args) => {
      const terminal = window.createTerminal({
        name: "Turbo Codemod",
        isTransient: true,
        iconPath: Uri.joinPath(context.extensionUri, "resources", "icon.svg")
      });
      terminal.sendText(`npx --yes @turbo/codemod ${args}`);
      terminal.show();
    })
  );

  context.subscriptions.push(
    commands.registerCommand("turbo.install", () => {
      const terminal = window.createTerminal({
        name: "Install Turbo",
        isTransient: true,
        iconPath: Uri.joinPath(context.extensionUri, "resources", "icon.svg")
      });
      terminal.sendText("npm i -g turbo && exit");
      terminal.show();

      return new Promise((resolve) => {
        const dispose = window.onDidCloseTerminal((terminal) => {
          if (terminal.name === "Install Turbo") {
            dispose.dispose();
            resolve(terminal.exitStatus?.code);
          }
        });
      });
    })
  );

  toolbar = window.createStatusBarItem(StatusBarAlignment.Left, 100);

  // If the extension is launched in debug mode then the debug server options are used
  // Otherwise the run options are used

  if (!installedTurboLspPath && !fs.existsSync(packagedLspPath)) {
    window.showInformationMessage(
      `The turbo LSP is not yet supported on your platform (${process.platform}-${process.arch})`
    );
    return;
  }

  const serverCommand = installedTurboLspPath ?? packagedLspPath;
  const serverArgs = installedTurboLspPath ? ["__internal_lsp"] : [];

  logs.appendLine(
    installedTurboLspPath
      ? `using installed turbo for LSP at ${installedTurboLspPath}`
      : `using packaged turbo LSP at ${packagedLspPath}`
  );

  const serverOptions: ServerOptions = {
    run: {
      command: serverCommand,
      args: serverArgs
    },
    debug: {
      command: serverCommand,
      args: serverArgs
    }
  };

  // Options to control the language client
  const clientOptions: LanguageClientOptions = {
    // Register the server for turbo json documents
    documentSelector: [
      { scheme: "file", pattern: "**/turbo.json" },
      { scheme: "file", pattern: "**/turbo.jsonc" },
      { scheme: "file", pattern: "**/package.json" }
    ]
  };

  // Create the language client and start the client.
  client = new LanguageClient(
    "turboLSP",
    "Turborepo Language Server",
    serverOptions,
    clientOptions
  );

  // Start the client. This will also launch the server
  client.start();
}

export function deactivate(): Thenable<void> | undefined {
  if (!client) {
    return undefined;
  }
  return client.stop();
}

function updateStatusBarItem(running: boolean) {
  toolbar.command = running ? "turbo.daemon.stop" : "turbo.daemon.start";
  toolbar.text = running ? "turbo Running" : "turbo Stopped";
  toolbar.show();
}

function quoteCommand(command: string) {
  return command.includes(" ") ? `"${command.replace(/"/g, '\\"')}"` : command;
}

function executableNames(name: string) {
  return process.platform === "win32"
    ? [`${name}.exe`, `${name}.cmd`, `${name}`]
    : [name];
}

function resolveTurboPath(
  turboPath: string | undefined,
  workspaceRoot?: string
) {
  if (!turboPath) {
    return undefined;
  }

  const resolvedPath = path.isAbsolute(turboPath)
    ? turboPath
    : path.resolve(workspaceRoot ?? process.cwd(), turboPath);

  if (!fs.existsSync(resolvedPath)) {
    logs.appendLine(
      `Manually specified turbo does not exist at path ${turboPath}`
    );
    return undefined;
  }

  if (fs.statSync(resolvedPath).isDirectory()) {
    return findTurboInDirectory(resolvedPath);
  }

  return resolvedPath;
}

function findTurboInDirectory(directory: string) {
  for (const executable of executableNames("turbo")) {
    const candidate = path.join(directory, executable);
    if (fs.existsSync(candidate) && !fs.statSync(candidate).isDirectory()) {
      return candidate;
    }
  }
}

function findExecutableOnPath(name: string) {
  const pathEntries = (process.env.PATH ?? "").split(path.delimiter);
  for (const pathEntry of pathEntries) {
    const executable = findTurboInDirectory(pathEntry);
    if (executable && path.basename(executable).startsWith(name)) {
      return executable;
    }
  }
}

function findTurbo(
  workspaceRoot: string | undefined,
  options: cp.ExecSyncOptionsWithStringEncoding
) {
  logs.appendLine("attempting to find turbo");
  return (
    findExecutableOnPath("turbo") ?? findLocalTurbo(workspaceRoot, options)
  );
}

function findInstalledTurboLsp(
  workspaceRoot: string | undefined,
  options: cp.ExecSyncOptionsWithStringEncoding,
  configuredTurboPath: string | undefined
) {
  logs.appendLine("resolving turbo LSP server");

  const candidates = [
    { label: "configured turbo.path", path: configuredTurboPath },
    {
      label: "workspace node_modules/.bin",
      path: workspaceRoot
        ? findTurboInDirectory(path.join(workspaceRoot, "node_modules", ".bin"))
        : undefined
    },
    { label: "PATH", path: findExecutableOnPath("turbo") }
  ];

  for (const candidate of candidates) {
    if (!candidate.path) {
      logs.appendLine(`turbo LSP: no candidate from ${candidate.label}`);
      continue;
    }

    logs.appendLine(
      `turbo LSP: probing ${candidate.label} at ${candidate.path}`
    );

    const probe = probeInternalLsp(candidate.path, options);
    if (probe.supported) {
      logs.appendLine(
        `turbo LSP: using ${candidate.label} at ${candidate.path}`
      );
      return candidate.path;
    }

    logs.appendLine(
      `turbo LSP: rejected ${candidate.label} at ${candidate.path}: ${probe.reason}`
    );
  }

  logs.appendLine(
    "turbo LSP: no installed turbo candidate supports internal LSP; falling back to packaged LSP binary"
  );
}

function probeInternalLsp(
  turboPath: string,
  options: cp.ExecSyncOptionsWithStringEncoding
): InternalLspProbeResult {
  try {
    const output = cp
      .execSync(`${quoteCommand(turboPath)} __internal_lsp --probe`, {
        ...options,
        stdio: ["ignore", "pipe", "pipe"],
        timeout: 1000
      })
      .trim();

    if (output === "turbo-lsp") {
      return { supported: true };
    }

    return { supported: false, reason: formatProbeOutput(output) };
  } catch (e) {
    return { supported: false, reason: formatProbeError(e) };
  }
}

function formatProbeOutput(output: string) {
  const line = firstNonEmptyLine(output);
  return line ? `unexpected probe output: ${line}` : "empty probe output";
}

function formatProbeError(error: unknown) {
  const stderr = outputFromError(error, "stderr");
  if (stderr) {
    return firstNonEmptyLine(stderr) ?? "probe command failed with stderr";
  }

  const stdout = outputFromError(error, "stdout");
  if (stdout) {
    return `probe command failed with stdout: ${firstNonEmptyLine(stdout)}`;
  }

  if (error instanceof Error && error.message) {
    return firstNonEmptyLine(error.message) ?? error.message;
  }

  return "probe command failed";
}

function outputFromError(error: unknown, key: "stdout" | "stderr") {
  if (typeof error !== "object" || error === null || !(key in error)) {
    return;
  }

  const output = (error as Record<string, unknown>)[key];
  if (Buffer.isBuffer(output)) {
    return output.toString("utf8").trim();
  }

  if (typeof output === "string") {
    return output.trim();
  }
}

function firstNonEmptyLine(output: string) {
  return output
    .split(/\r?\n/)
    .map((line) => line.trim())
    .find(Boolean);
}

async function promptGlobalTurbo(useLocalTurbo: boolean) {
  if (useLocalTurbo) {
    return;
  }

  const answer = await window.showErrorMessage(
    "turbo not found. Please see the docs to install, or set the path manually in the settings.",
    "Install Now",
    "Open Docs",
    "Open Settings"
  );

  if (answer === "Install Now") {
    const exitCode = await commands.executeCommand("turbo.install");
    if (exitCode === 0) {
      window.showInformationMessage("turbo installed");
      await commands.executeCommand("turbo.daemon.start");
    } else {
      const message = await window.showErrorMessage(
        "Unable to install turbo. Please install manually.",
        "Open Docs"
      );

      if (message === "Open Docs") {
        env.openExternal(Uri.parse("https://turborepo.dev/docs/installing"));
      }
    }
  } else if (answer === "Open Docs") {
    env.openExternal(Uri.parse("https://turborepo.dev/docs/installing"));
  } else if (answer === "Open Settings") {
    commands.executeCommand("workbench.action.openSettings", "turbo.path");
  }
}

function findLocalTurbo(
  workspaceRoot: string | undefined,
  options: cp.ExecSyncOptionsWithStringEncoding
): string | undefined {
  const checks = [
    () => {
      if (workspaceRoot) {
        logs.appendLine("attempting to find local turbo in node_modules/.bin");
        return findTurboInDirectory(
          path.join(workspaceRoot, "node_modules", ".bin")
        );
      }
    },
    () => {
      logs.appendLine("attempting to find local turbo using npm");
      const npmList = cp.execSync("npm ls turbo --json", options);
      const npmData = JSON.parse(npmList);

      // this is relative to node_modules
      const packagePath = npmData?.dependencies?.turbo?.resolved;

      const PREFIX = "file:"; // npm ls returns a file: prefix

      if (packagePath?.startsWith(PREFIX)) {
        const turboPath = path.join(
          "node_modules",
          packagePath.slice(PREFIX.length),
          "bin",
          "turbo"
        );
        return resolveTurboPath(turboPath, workspaceRoot);
      }
    },
    () => {
      logs.appendLine("attempting to find local turbo using yarn");
      const turboBin = cp.execSync("yarn bin turbo", options);
      return resolveTurboPath(turboBin.trim(), workspaceRoot);
    },
    () => {
      logs.appendLine("attempting to find local turbo using pnpm");
      const binFolder = cp.execSync("pnpm bin", options).trim();
      return findTurboInDirectory(binFolder);
    },
    () => {
      logs.appendLine("attempting to find local turbo using bun");
      const binFolder = cp.execSync("bun pm bin", options).trim();
      return findTurboInDirectory(binFolder);
    }
  ];

  for (const potentialPath of checks) {
    try {
      const potential = potentialPath()?.trim();
      if (potential && fs.existsSync(potential)) {
        logs.appendLine(`found local turbo at ${potential}`);
        return potential;
      }
    } catch (e) {
      // no-op
    }
  }
}

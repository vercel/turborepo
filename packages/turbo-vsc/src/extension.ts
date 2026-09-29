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
import * as fs from "node:fs";

import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions
} from "vscode-languageclient/node";

import { createTurboDaemonArgs } from "./turbo-daemon-command";
import {
  findInstalledTurboLsp,
  findTurbo,
  resolveTurboPath
} from "./turbo-discovery";
import {
  createTurboRunTerminalOptions,
  sanitizeTurboRunTaskName
} from "./turbo-run-terminal-options";

let client: LanguageClient;

let toolbar: StatusBarItem;

const logs = window.createOutputChannel("Turborepo Extension");

export function activate(context: ExtensionContext) {
  if (!workspace.isTrusted) {
    window.showWarningMessage(
      "The Turborepo extension is disabled in untrusted workspaces."
    );
    return;
  }

  const workspaceRoot = workspace.workspaceFolders?.[0]?.uri.fsPath;
  const options: cp.ExecFileOptions = { cwd: workspaceRoot };

  const turboSettings = workspace.getConfiguration("turbo");
  const configuredTurboPath: string | undefined = turboSettings.get("path");
  const useLocalTurbo: boolean = turboSettings.get("useLocalTurbo") ?? false;

  const log = (message: string) => logs.appendLine(message);

  logs.appendLine("starting the turbo extension");

  // Discovery spawns child processes with bounded timeouts; abort any
  // in-flight probes when the extension deactivates so slow binaries and
  // package-manager shims are killed instead of lingering.
  const discoveryAbort = new AbortController();
  context.subscriptions.push({
    dispose: () => {
      discoveryAbort.abort();
    }
  });

  let turboPath = resolveTurboPath(configuredTurboPath, workspaceRoot, log);

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

  // The probe runs asynchronously on the extension host's event loop. Command
  // handlers and LSP startup await the shared promise, so activation never
  // blocks on slow binaries.
  const installedTurboLspPromise = findInstalledTurboLsp({
    workspaceRoot,
    configuredTurboPath: turboPath,
    signal: discoveryAbort.signal,
    log
  }).catch((err) => {
    if (!discoveryAbort.signal.aborted) {
      logs.appendLine(`turbo LSP discovery failed: ${err}`);
    }
    return undefined;
  });

  const getTurboPath = async () => {
    turboPath ??= await findTurbo({
      workspaceRoot,
      signal: discoveryAbort.signal,
      log
    });
    if (turboPath) {
      return turboPath;
    }

    await promptGlobalTurbo(useLocalTurbo);
    turboPath = await findTurbo({
      workspaceRoot,
      signal: discoveryAbort.signal,
      log
    });
    return turboPath;
  };

  const getDaemonCommandPath = async () => {
    const daemonCommandPath =
      (await installedTurboLspPromise) ?? packagedLspPath;
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

      cp.execFile(
        daemonPath,
        createTurboDaemonArgs("start"),
        options,
        (err) => {
          if (err) {
            if (isCommandNotFoundError(err)) {
              promptGlobalTurbo(useLocalTurbo);
            } else {
              logs.appendLine(`unable to start turbo: ${err.message}`);
            }
          } else {
            updateStatusBarItem(true);
            window.showInformationMessage("Turbo daemon started");
          }
        }
      );
    })
  );

  context.subscriptions.push(
    commands.registerCommand("turbo.daemon.stop", async () => {
      const daemonPath = await getDaemonCommandPath();
      if (!daemonPath) {
        return;
      }

      cp.execFile(daemonPath, createTurboDaemonArgs("stop"), options, (err) => {
        if (err) {
          if (isCommandNotFoundError(err)) {
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

      cp.execFile(
        daemonPath,
        createTurboDaemonArgs("status"),
        options,
        (err) => {
          if (err) {
            if (isCommandNotFoundError(err)) {
              promptGlobalTurbo(useLocalTurbo);
            } else {
              logs.appendLine(`unable to get turbo status: ${err.message}`);
              updateStatusBarItem(false);
            }
          } else {
            updateStatusBarItem(true);
            window.showInformationMessage("Turbo daemon is running");
          }
        }
      );
    })
  );

  context.subscriptions.push(
    commands.registerCommand("turbo.run", async (args: unknown) => {
      const taskName = sanitizeTurboRunTaskName(args);
      if (!taskName) {
        window.showWarningMessage("Invalid Turborepo task name.");
        return;
      }

      const turboPath = await getTurboPath();
      if (!turboPath) {
        return;
      }

      const terminal = window.createTerminal({
        ...createTurboRunTerminalOptions(turboPath, taskName),
        iconPath: Uri.joinPath(context.extensionUri, "resources", "icon.svg")
      });
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

  void (async () => {
    const installedTurboLspPath = await installedTurboLspPromise;
    if (discoveryAbort.signal.aborted) {
      return;
    }

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
  })();
}

export function deactivate(): Thenable<void> | undefined {
  if (!client) {
    return undefined;
  }
  return client.stop();
}

function isCommandNotFoundError(err: Error): boolean {
  return (
    err.message.includes("command not found") || err.message.includes("ENOENT")
  );
}

function updateStatusBarItem(running: boolean) {
  toolbar.command = running ? "turbo.daemon.stop" : "turbo.daemon.start";
  toolbar.text = running ? "turbo Running" : "turbo Stopped";
  toolbar.show();
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

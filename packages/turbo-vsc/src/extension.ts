import {
  ExtensionContext,
  window,
  commands,
  workspace,
  StatusBarAlignment,
  StatusBarItem,
  TextEditor,
  Range,
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

import { getTaskDefinitionKeyDecorationOffsets } from "./json-decorations";

let client: LanguageClient;

let toolbar: StatusBarItem;

// thunks passed to this function will executed
// after no calls have been made for `waitMs` milliseconds
const useDebounce = <T>(func: (args: T) => void, waitMs: number) => {
  let timeout: any;
  return (args: T) => {
    clearTimeout(timeout);
    timeout = setTimeout(() => {
      func(args);
    }, waitMs);
  };
};

const logs = window.createOutputChannel("Turborepo Extension");

function rainbowRgb(i: number) {
  const f = 0.5;
  const r = Math.sin(f * i + (4 * Math.PI) / 3) * 127 + 128;
  const g = 45;
  const b = Math.sin(f * i) * 127 + 128;

  return `#${Math.round(r).toString(16).padStart(2, "0")}${Math.round(g)
    .toString(16)
    .padStart(2, "0")}${Math.round(b).toString(16).padStart(2, "0")}`;
}

const taskDefinitionColors = [...Array(10).keys()]
  .map(rainbowRgb)
  .map((color) =>
    window.createTextEditorDecorationType({
      color
    })
  );

const refreshDecorations = useDebounce(updateJSONDecorations, 1000);

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

  // decorate when changing the active editor editor
  context.subscriptions.push(
    window.onDidChangeActiveTextEditor(
      (editor) => updateJSONDecorations(editor),
      null,
      context.subscriptions
    )
  );

  // decorate when the document changes
  context.subscriptions.push(
    workspace.onDidChangeTextDocument(
      (event) => {
        if (
          window.activeTextEditor &&
          event.document === window.activeTextEditor.document
        ) {
          refreshDecorations(window.activeTextEditor);
        }
      },
      null,
      context.subscriptions
    )
  );

  // decorate the active editor now
  updateJSONDecorations(window.activeTextEditor);

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

function updateJSONDecorations(editor?: TextEditor) {
  if (
    !editor ||
    !path.basename(editor.document.fileName).endsWith("turbo.json")
  ) {
    return;
  }

  const decorationOffsets = getTaskDefinitionKeyDecorationOffsets(
    editor.document.getText()
  );

  for (let i = 0; i < decorationOffsets.length; i++) {
    const index = decorationOffsets[i];
    editor.setDecorations(taskDefinitionColors[i + 1], [
      new Range(
        editor.document.positionAt(index),
        editor.document.positionAt(index + 1)
      )
    ]);
  }
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

    if (supportsInternalLsp(candidate.path, options)) {
      logs.appendLine(
        `turbo LSP: using ${candidate.label} at ${candidate.path}`
      );
      return candidate.path;
    }

    logs.appendLine(
      `turbo LSP: ${candidate.label} does not support internal LSP`
    );
  }

  logs.appendLine("turbo LSP: falling back to packaged LSP binary");
}

function supportsInternalLsp(
  turboPath: string,
  options: cp.ExecSyncOptionsWithStringEncoding
) {
  try {
    return (
      cp
        .execSync(`${quoteCommand(turboPath)} __internal_lsp --probe`, {
          ...options,
          timeout: 1000
        })
        .trim() === "turbo-lsp"
    );
  } catch (e) {
    return false;
  }
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

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
  env,
} from "vscode";
import * as cp from "child_process";
import * as path from "path";
import * as fs from "fs";

import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
} from "vscode-languageclient/node";

import { visit } from "jsonc-parser";

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
  const r = Math.sin(f * i + (4.0 * Math.PI) / 3.0) * 127.0 + 128.0;
  const g = 45;
  const b = Math.sin(f * i) * 127.0 + 128.0;

  return `#${Math.round(r).toString(16).padStart(2, "0")}${Math.round(g)
    .toString(16)
    .padStart(2, "0")}${Math.round(b).toString(16).padStart(2, "0")}`;
}

const pipelineColors = [...Array(10).keys()].map(rainbowRgb).map((color) =>
  window.createTextEditorDecorationType({
    color,
  })
);

const refreshDecorations = useDebounce(updateJSONDecorations, 1000);

export function activate(context: ExtensionContext) {
  const options: cp.ExecSyncOptionsWithStringEncoding = {
    cwd: workspace.workspaceFolders?.[0].uri.path,
    encoding: "utf8",
  };

  const turboSettings = workspace.getConfiguration("turbo");
  let turboPath: string | undefined = turboSettings.get("path");
  const useLocalTurbo: boolean = turboSettings.get("useLocalTurbo") ?? false;

  logs.appendLine("starting the turbo extension");

  if (turboPath && !fs.existsSync(turboPath)) {
    logs.appendLine(
      `manually specified turbo does not exist at path \`${turboPath}\`, attempting to locate it`
    );
    turboPath = undefined;
  }

  try {
    if (turboPath == null) {
      logs.appendLine("attempting to find global turbo");
      turboPath = cp
        .execSync(
          // attempt to source two well known version managers
          // as well as adding the bun global bin to the path
          'bash -c \'source "$HOME/.nvm/nvm.sh" > /dev/null 2>&1; source "$HOME/.asdf/asdf.sh" > /dev/null 2>&1; eval "$(brew shellenv)" > /dev/null 2>&1; export PATH="$HOME/.bun/bin:$PATH"; which turbo\'',
          options
        )
        .trim();
      logs.appendLine(`set turbo path to ${turboPath}`);
    }
  } catch (e: any) {
    if (
      e.message.includes("command not found") ||
      e.message.includes("Command failed") ||
      e.message.includes("which: no turbo in")
    ) {
      // attempt to find local turbo instead
      logs.appendLine("prompting global turbo");
      promptGlobalTurbo(useLocalTurbo);
      turboPath = findLocalTurbo();
    } else {
      logs.appendLine(`unable to find turbo: ${e.message}`);
    }
  }

  if (turboPath) {
    logs.appendLine(`using turbo at path ${turboPath}`);
  }

  context.subscriptions.push(
    commands.registerCommand("turbo.daemon.start", () => {
      cp.exec(`${turboPath} daemon start`, options, (err) => {
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
    commands.registerCommand("turbo.daemon.stop", () => {
      cp.exec(`${turboPath} daemon stop`, options, (err) => {
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
    commands.registerCommand("turbo.daemon.status", () => {
      cp.exec(`${turboPath} daemon status`, options, (err) => {
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
    commands.registerCommand("turbo.run", (args) => {
      const terminal = window.createTerminal({
        name: `${args}`,
        isTransient: true,
        iconPath: Uri.joinPath(context.extensionUri, "resources", "icon.svg"),
      });
      terminal.sendText(`${turboPath} run ${args}`);
      terminal.show();
    })
  );

  context.subscriptions.push(
    commands.registerCommand("turbo.codemod", (args) => {
      const terminal = window.createTerminal({
        name: "Turbo Codemod",
        isTransient: true,
        iconPath: Uri.joinPath(context.extensionUri, "resources", "icon.svg"),
      });
      terminal.sendText(`npx --yes @turbo/codemod ${args}`);
      terminal.show();
    })
  );

  context.subscriptions.push(
    commands.registerCommand("turbo.install", (args) => {
      const terminal = window.createTerminal({
        name: "Install Turbo",
        isTransient: true,
        iconPath: Uri.joinPath(context.extensionUri, "resources", "icon.svg"),
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
  commands.executeCommand("turbo.daemon.start");

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

  const lspPath = Uri.joinPath(
    context.extensionUri,
    "out",
    `turborepo-lsp-${process.platform}-${process.arch}${
      process.platform === "win32" ? ".exe" : ""
    }`
  ).fsPath;

  if (!fs.existsSync(lspPath)) {
    window.showInformationMessage(
      `The turbo LSP is not yet supported on your platform (${process.platform}-${process.arch})`
    );
    return;
  }

  const serverOptions: ServerOptions = {
    run: {
      command: lspPath,
    },
    debug: {
      command: lspPath,
    },
  };

  // Options to control the language client
  const clientOptions: LanguageClientOptions = {
    // Register the server for turbo json documents
    documentSelector: [
      { scheme: "file", pattern: "**/turbo.json" },
      { scheme: "file", pattern: "**/package.json" },
    ],
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

  let depth = 0; // indicates we're not in a pipeline block
  let inPipeline = false; // we do not use this right now but could highlight tasks

  visit(editor.document.getText(), {
    onObjectProperty: (property, offset) => {
      // only highlight pipeline at the top level
      if (property === "pipeline" && depth === 0 && !inPipeline) {
        inPipeline = true;
        for (let i = 1; i < 9; i++) {
          const index = i + offset;
          editor.setDecorations(pipelineColors[i], [
            new Range(
              editor.document.positionAt(index),
              editor.document.positionAt(index + 1)
            ),
          ]);
        }
      }
    },
    onObjectBegin: () => {
      if (depth === -1) {
        depth = 0;
      } else if (depth !== -1) {
        depth += 1;
      }
    },
    onObjectEnd: () => {
      if (depth < -1) {
        depth -= 1;
      } else {
        throw Error("imbalanced visitor");
      }
    },
  });
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
        env.openExternal(Uri.parse("https://turbo.build/repo/docs/installing"));
      }
    }
  } else if (answer === "Open Docs") {
    env.openExternal(Uri.parse("https://turbo.build/repo/docs/installing"));
  } else if (answer === "Open Settings") {
    commands.executeCommand("workbench.action.openSettings", "turbo.path");
  }
}

function findLocalTurbo(): string | undefined {
  const options: cp.ExecSyncOptionsWithStringEncoding = {
    encoding: "utf8",
    cwd: workspace.workspaceFolders?.[0].uri.path,
  };

  const checks = [
    () => {
      logs.appendLine("attempting to find local turbo using npm");
      const npmList = cp.execSync("npm ls turbo --json", options);
      const npmData = JSON.parse(npmList);

      // this is relative to node_modules
      const packagePath = npmData?.dependencies?.turbo?.resolved;

      const PREFIX = "file:"; // npm ls returns a file: prefix

      if (packagePath?.startsWith(PREFIX)) {
        return path.join(
          "node_modules",
          packagePath.slice(PREFIX.length),
          "bin",
          "turbo"
        );
      }
    },
    () => {
      logs.appendLine("attempting to find local turbo using yarn");
      const turboBin = cp.execSync("yarn bin turbo", options);
      return turboBin.trim();
    },
    () => {
      logs.appendLine("attempting to find local turbo using pnpm");
      const binFolder = cp.execSync("pnpm bin", options).trim();
      return path.join(binFolder, "turbo");
    },
    () => {
      logs.appendLine("attempting to find local turbo using bun");
      const binFolder = cp.execSync("bun pm bin", options).trim();
      return path.join(binFolder, "turbo");
    },
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

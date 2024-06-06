// eslint-disable-next-line camelcase -- This is a test file
import child_process, {
  type ChildProcess,
  type ExecException,
} from "node:child_process";
import {
  spyConsole,
  spyExit,
  type SpyExit,
  mockEnv,
  validateLogs,
} from "@turbo/test-utils";
import { TurboIgnoreTelemetry, TelemetryConfig } from "@turbo/telemetry";
import { turboIgnore } from "../src/ignore";

function expectBuild(mockExit: SpyExit) {
  expect(mockExit.exit).toHaveBeenCalledWith(1);
}

function expectIgnore(mockExit: SpyExit) {
  expect(mockExit.exit).toHaveBeenCalledWith(0);
}

describe("turboIgnore()", () => {
  mockEnv();
  const mockExit = spyExit();
  const mockConsole = spyConsole();

  const telemetry = new TurboIgnoreTelemetry({
    api: "https://example.com",
    packageInfo: {
      name: "turbo-ignore",
      version: "1.0.0",
    },
    config: new TelemetryConfig({
      configPath: "test-config-path",
      config: {
        telemetry_enabled: false,
        telemetry_id: "telemetry-test-id",
        telemetry_salt: "telemetry-salt",
      },
    }),
  });

  it("throws error and allows build when exec fails", () => {
    const mockExec = jest
      .spyOn(child_process, "exec")
      .mockImplementation((command, options, callback) => {
        if (callback) {
          return callback(
            { message: "error details" } as unknown as ExecException,
            "stdout",
            "stderr"
          ) as unknown as ChildProcess;
        }
        return {} as unknown as ChildProcess;
      });

    turboIgnore("test-workspace", { telemetry });

    expect(mockExec).toHaveBeenCalledWith(
      `npx -y turbo@^2 run build --filter="test-workspace...[HEAD^]" --dry=json`,
      expect.anything(),
      expect.anything()
    );

    validateLogs(["UNKNOWN_ERROR: error details"], mockConsole.error, {
      prefix: "≫  ",
    });

    expectBuild(mockExit);
    mockExec.mockRestore();
  });

  it("throws pretty error and allows build when exec fails", () => {
    const mockExec = jest
      .spyOn(child_process, "exec")
      .mockImplementation((command, options, callback) => {
        if (callback) {
          return callback(
            {
              message:
                "run failed: We did not detect an in-use package manager for your project",
            } as unknown as ExecException,
            "stdout",
            "stderr"
          ) as unknown as ChildProcess;
        }
        return {} as unknown as ChildProcess;
      });

    turboIgnore("test-workspace", {});

    expect(mockExec).toHaveBeenCalledWith(
      `npx -y turbo@^2 run build --filter="test-workspace...[HEAD^]" --dry=json`,
      expect.anything(),
      expect.anything()
    );

    validateLogs(
      [
        `turbo-ignore could not complete - no package manager detected, please commit a lockfile, or set "packageManager" in your root "package.json"`,
      ],
      mockConsole.warn,
      { prefix: "≫  " }
    );

    expectBuild(mockExit);
    mockExec.mockRestore();
  });

  it("throws pretty error and allows build when can't find previous sha", () => {
    process.env.VERCEL = "1";
    process.env.VERCEL_GIT_PREVIOUS_SHA = "too-far-back";
    process.env.VERCEL_GIT_COMMIT_REF = "my-branch";

    const mockExecSync = jest
      .spyOn(child_process, "execSync")
      .mockReturnValue("commit");

    const mockExec = jest
      .spyOn(child_process, "exec")
      .mockImplementation((command, options, callback) => {
        if (callback) {
          return callback(
            {
              message:
                " ERROR  run failed: failed to resolve packages to run: commit too-far-back does not exist",
            } as unknown as ExecException,
            "stdout",
            "stderr"
          ) as unknown as ChildProcess;
        }
        return {} as unknown as ChildProcess;
      });

    turboIgnore("test-workspace", { telemetry });

    expect(mockExec).toHaveBeenCalledWith(
      `npx -y turbo@^2 run build --filter="test-workspace...[too-far-back]" --dry=json`,
      expect.anything(),
      expect.anything()
    );

    validateLogs(
      [
        `turbo-ignore could not complete - a ref or SHA is invalid. It could have been removed from the branch history via a force push, or this could be a shallow clone with insufficient history`,
      ],
      mockConsole.warn,
      { prefix: "≫  " }
    );

    expectBuild(mockExit);
    mockExecSync.mockRestore();
    mockExec.mockRestore();
  });

  it("throws pretty error and allows build when fallback fails", () => {
    const mockExec = jest
      .spyOn(child_process, "exec")
      .mockImplementation((command, options, callback) => {
        if (callback) {
          return callback(
            {
              message:
                "ERROR run failed: failed to resolve packages to run: commit HEAD^ does not exist",
            } as unknown as ExecException,
            "stdout",
            "stderr"
          ) as unknown as ChildProcess;
        }
        return {} as unknown as ChildProcess;
      });

    turboIgnore("test-workspace", { fallback: "HEAD^" });

    expect(mockExec).toHaveBeenCalledWith(
      `npx -y turbo@^2 run build --filter="test-workspace...[HEAD^]" --dry=json`,
      expect.anything(),
      expect.anything()
    );

    validateLogs(
      [
        `turbo-ignore could not complete - parent commit does not exist or is unreachable`,
      ],
      mockConsole.warn,
      { prefix: "≫  " }
    );

    expectBuild(mockExit);
    mockExec.mockRestore();
  });

  it("skips checks and allows build when no workspace can be found", () => {
    turboIgnore(undefined, { directory: "__fixtures__/no-app" });
    validateLogs(
      [
        () => [
          "≫  ",
          expect.stringContaining(
            " could not be found. turbo-ignore inferencing failed"
          ),
        ],
      ],
      mockConsole.error,
      { prefix: "≫  " }
    );
    expectBuild(mockExit);
  });

  it("skips checks and allows build when a workspace with no name is found", () => {
    turboIgnore(undefined, { directory: "__fixtures__/invalid-app" });
    validateLogs(
      [
        () => [
          "≫  ",
          expect.stringContaining(' is missing the "name" field (required).'),
        ],
      ],
      mockConsole.error,
      { prefix: "≫  " }
    );
    expectBuild(mockExit);
  });

  it("skips checks and allows build when no monorepo root can be found", () => {
    turboIgnore(undefined, { directory: "/" });
    expectBuild(mockExit);
    expect(mockConsole.error).toHaveBeenLastCalledWith(
      "≫  ",
      "Monorepo root not found. turbo-ignore inferencing failed"
    );
  });

  it("skips checks and allows build when TURBO_FORCE is set", () => {
    process.env.TURBO_FORCE = "true";
    turboIgnore(undefined, { directory: "test-workspace" });
    expect(mockConsole.log).toHaveBeenNthCalledWith(
      2,
      "≫  ",
      "`TURBO_FORCE` detected"
    );
    expectBuild(mockExit);
  });

  it("allows build when no comparison is returned", () => {
    process.env.VERCEL = "1";
    process.env.VERCEL_GIT_PREVIOUS_SHA = "";
    process.env.VERCEL_GIT_COMMIT_REF = "my-branch";
    turboIgnore("test-app", { directory: "__fixtures__/app" });
    expect(mockConsole.log).toHaveBeenNthCalledWith(
      5,
      "≫  ",
      'No previous deployments found for "test-app" on branch "my-branch"'
    );
    expectBuild(mockExit);
  });

  it("skips build for `previousDeploy` comparison with no changes", () => {
    process.env.VERCEL = "1";
    process.env.VERCEL_GIT_PREVIOUS_SHA = "last-deployed-sha";
    process.env.VERCEL_GIT_COMMIT_REF = "my-branch";

    const mockExecSync = jest
      .spyOn(child_process, "execSync")
      .mockReturnValue("commit");

    const mockExec = jest
      .spyOn(child_process, "exec")
      .mockImplementation((command, options, callback) => {
        if (callback) {
          return callback(
            null,
            '{"packages":[],"tasks":[]}',
            "stderr"
          ) as unknown as ChildProcess;
        }
        return {} as unknown as ChildProcess;
      });
    turboIgnore(undefined, { directory: "__fixtures__/app" });
    validateLogs(
      [
        "Using Turborepo to determine if this project is affected by the commit...\n",
        'Inferred "test-app" as workspace from "package.json"',
        'Inferred turbo version ^2 based on "tasks" in "turbo.json"',
        'Using "build" as the task as it was unspecified',
        `Found previous deployment ("last-deployed-sha") for "test-app" on branch "my-branch"`,
        'Analyzing results of `npx -y turbo@^2 run build --filter="test-app...[last-deployed-sha]" --dry=json`',
        "This project and its dependencies are not affected",
        () => expect.stringContaining("⏭ Ignoring the change"),
      ],
      mockConsole.log,
      { prefix: "≫  " }
    );

    expectIgnore(mockExit);
    mockExecSync.mockRestore();
    mockExec.mockRestore();
  });

  it("allows build for `previousDeploy` comparison with changes", () => {
    process.env.VERCEL = "1";
    process.env.VERCEL_GIT_PREVIOUS_SHA = "last-deployed-sha";
    process.env.VERCEL_GIT_COMMIT_REF = "my-branch";

    const mockExecSync = jest
      .spyOn(child_process, "execSync")
      .mockReturnValue("commit");

    const mockExec = jest
      .spyOn(child_process, "exec")
      .mockImplementation((command, options, callback) => {
        if (callback) {
          return callback(
            null,
            '{"packages":["test-app"],"tasks":[]}',
            "stderr"
          ) as unknown as ChildProcess;
        }
        return {} as unknown as ChildProcess;
      });
    turboIgnore(undefined, {
      task: "workspace#build",
      directory: "__fixtures__/app",
    });
    validateLogs(
      [
        "Using Turborepo to determine if this project is affected by the commit...\n",
        'Inferred "test-app" as workspace from "package.json"',
        'Inferred turbo version ^2 based on "tasks" in "turbo.json"',
        'Using "workspace#build" as the task from the arguments',
        'Found previous deployment ("last-deployed-sha") for "test-app" on branch "my-branch"',
        'Analyzing results of `npx -y turbo@^2 run "workspace#build" --filter="test-app...[last-deployed-sha]" --dry=json`',
        'This commit affects "test-app"',
        () => expect.stringContaining("✓ Proceeding with deployment"),
      ],
      mockConsole.log,
      { prefix: "≫  " }
    );

    expectBuild(mockExit);
    mockExecSync.mockRestore();
    mockExec.mockRestore();
  });

  it("allows build for `previousDeploy` comparison with single dependency change", () => {
    process.env.VERCEL = "1";
    process.env.VERCEL_GIT_PREVIOUS_SHA = "last-deployed-sha";
    process.env.VERCEL_GIT_COMMIT_REF = "my-branch";

    const mockExecSync = jest
      .spyOn(child_process, "execSync")
      .mockReturnValue("commit");

    const mockExec = jest
      .spyOn(child_process, "exec")
      .mockImplementation((command, options, callback) => {
        if (callback) {
          return callback(
            null,
            '{"packages":["test-app", "ui"],"tasks":[]}',
            "stderr"
          ) as unknown as ChildProcess;
        }
        return {} as unknown as ChildProcess;
      });
    turboIgnore(undefined, { directory: "__fixtures__/app" });
    validateLogs(
      [
        "Using Turborepo to determine if this project is affected by the commit...\n",
        'Inferred "test-app" as workspace from "package.json"',
        'Inferred turbo version ^2 based on "tasks" in "turbo.json"',
        'Using "build" as the task as it was unspecified',
        'Found previous deployment ("last-deployed-sha") for "test-app" on branch "my-branch"',
        'Analyzing results of `npx -y turbo@^2 run build --filter="test-app...[last-deployed-sha]" --dry=json`',
        'This commit affects "test-app" and 1 dependency (ui)',
        () => expect.stringContaining("✓ Proceeding with deployment"),
      ],
      mockConsole.log,
      { prefix: "≫  " }
    );

    expectBuild(mockExit);
    mockExecSync.mockRestore();
    mockExec.mockRestore();
  });

  it("allows build for `previousDeploy` comparison with multiple dependency changes", () => {
    process.env.VERCEL = "1";
    process.env.VERCEL_GIT_PREVIOUS_SHA = "last-deployed-sha";
    process.env.VERCEL_GIT_COMMIT_REF = "my-branch";

    const mockExecSync = jest
      .spyOn(child_process, "execSync")
      .mockReturnValue("commit");

    const mockExec = jest
      .spyOn(child_process, "exec")
      .mockImplementation((command, options, callback) => {
        if (callback) {
          return callback(
            null,
            '{"packages":["test-app", "ui", "tsconfig"],"tasks":[]}',
            "stderr"
          ) as unknown as ChildProcess;
        }
        return {} as unknown as ChildProcess;
      });
    turboIgnore(undefined, { directory: "__fixtures__/app" });
    validateLogs(
      [
        "Using Turborepo to determine if this project is affected by the commit...\n",
        'Inferred "test-app" as workspace from "package.json"',
        'Inferred turbo version ^2 based on "tasks" in "turbo.json"',
        'Using "build" as the task as it was unspecified',
        'Found previous deployment ("last-deployed-sha") for "test-app" on branch "my-branch"',
        'Analyzing results of `npx -y turbo@^2 run build --filter="test-app...[last-deployed-sha]" --dry=json`',
        'This commit affects "test-app" and 2 dependencies (ui, tsconfig)',
        () => expect.stringContaining("✓ Proceeding with deployment"),
      ],
      mockConsole.log,
      { prefix: "≫  " }
    );

    expectBuild(mockExit);
    mockExecSync.mockRestore();
    mockExec.mockRestore();
  });

  it("allows build for unavailable `previousDeploy` comparison with fallback", () => {
    process.env.VERCEL = "1";
    process.env.VERCEL_GIT_PREVIOUS_SHA = "last-deployed-sha";
    process.env.VERCEL_GIT_COMMIT_REF = "my-branch";

    const mockExecSync = jest
      .spyOn(child_process, "execSync")
      .mockImplementation((cmd: string) => {
        if (cmd.includes("git cat-file")) {
          throw new Error("fatal: Not a valid object name last-deployed-sha");
        }
        return "";
      });

    const mockExec = jest
      .spyOn(child_process, "exec")
      .mockImplementation((command, options, callback) => {
        if (callback) {
          return callback(
            null,
            '{"packages":["test-app"],"tasks":[]}',
            "stderr"
          ) as unknown as ChildProcess;
        }
        return {} as unknown as ChildProcess;
      });
    turboIgnore(undefined, {
      task: "workspace#build",
      fallback: "HEAD^2",
      directory: "__fixtures__/app",
    });
    validateLogs(
      [
        "Using Turborepo to determine if this project is affected by the commit...\n",
        'Inferred "test-app" as workspace from "package.json"',
        'Inferred turbo version ^2 based on "tasks" in "turbo.json"',
        'Using "workspace#build" as the task from the arguments',
        'Previous deployment ("last-deployed-sha") for "test-app" on branch "my-branch" is unreachable.',
        "Falling back to ref HEAD^2",
        'Analyzing results of `npx -y turbo@^2 run "workspace#build" --filter="test-app...[HEAD^2]" --dry=json`',
        'This commit affects "test-app"',
        () => expect.stringContaining("✓ Proceeding with deployment"),
      ],
      mockConsole.log,
      { prefix: "≫  " }
    );

    expectBuild(mockExit);
    mockExecSync.mockRestore();
    mockExec.mockRestore();
  });

  it("throws error and allows build when json cannot be parsed", () => {
    const mockExec = jest
      .spyOn(child_process, "exec")
      .mockImplementation((command, options, callback) => {
        if (callback) {
          return callback(null, "stdout", "stderr") as unknown as ChildProcess;
        }
        return {} as unknown as ChildProcess;
      });

    turboIgnore(undefined, { directory: "__fixtures__/app" });

    expect(mockExec).toHaveBeenCalledWith(
      `npx -y turbo@^2 run build --filter="test-app...[HEAD^]" --dry=json`,
      expect.anything(),
      expect.anything()
    );
    validateLogs(
      [
        'Failed to parse JSON output from `npx -y turbo@^2 run build --filter="test-app...[HEAD^]" --dry=json`.',
      ],
      mockConsole.error,
      { prefix: "≫  " }
    );

    expectBuild(mockExit);
    mockExec.mockRestore();
  });

  it("throws error and allows build when stdout is null", () => {
    const mockExec = jest
      .spyOn(child_process, "exec")
      .mockImplementation((command, options, callback) => {
        if (callback) {
          return callback(
            null,
            null as unknown as string,
            "stderr"
          ) as unknown as ChildProcess;
        }
        return {} as unknown as ChildProcess;
      });

    turboIgnore(undefined, { directory: "__fixtures__/app" });

    expect(mockExec).toHaveBeenCalledWith(
      `npx -y turbo@^2 run build --filter="test-app...[HEAD^]" --dry=json`,
      expect.anything(),
      expect.anything()
    );
    validateLogs(
      [
        'Failed to parse JSON output from `npx -y turbo@^2 run build --filter="test-app...[HEAD^]" --dry=json`.',
      ],
      mockConsole.error,
      { prefix: "≫  " }
    );

    expectBuild(mockExit);
    mockExec.mockRestore();
  });

  it("skips when commit message contains a skip string", () => {
    process.env.VERCEL = "1";
    process.env.VERCEL_GIT_COMMIT_MESSAGE = "[vercel skip]";

    turboIgnore(undefined, { directory: "__fixtures__/app" });

    validateLogs(
      [
        "Using Turborepo to determine if this project is affected by the commit...\n",
        'Inferred "test-app" as workspace from "package.json"',
        'Inferred turbo version ^2 based on "tasks" in "turbo.json"',
        'Using "build" as the task as it was unspecified',
        "Found commit message: [vercel skip]",
        () => expect.stringContaining("⏭ Ignoring the change"),
      ],
      mockConsole.log,
      { prefix: "≫  " }
    );

    expectIgnore(mockExit);
  });

  it("deploys when commit message contains a deploy string", () => {
    process.env.VERCEL = "1";
    process.env.VERCEL_GIT_COMMIT_MESSAGE = "[vercel deploy]";

    turboIgnore(undefined, { directory: "__fixtures__/app" });

    validateLogs(
      [
        "Using Turborepo to determine if this project is affected by the commit...\n",
        'Inferred "test-app" as workspace from "package.json"',
        'Inferred turbo version ^2 based on "tasks" in "turbo.json"',
        'Using "build" as the task as it was unspecified',
        "Found commit message: [vercel deploy]",
        () => expect.stringContaining("✓ Proceeding with deployment"),
      ],
      mockConsole.log,
      { prefix: "≫  " }
    );

    expectBuild(mockExit);
  });

  it("runs full turbo-ignore check when commit message contains a conflicting string", () => {
    process.env.VERCEL = "1";
    process.env.VERCEL_GIT_COMMIT_MESSAGE = "[vercel deploy] [vercel skip]";
    process.env.VERCEL_GIT_PREVIOUS_SHA = "last-deployed-sha";
    process.env.VERCEL_GIT_COMMIT_REF = "my-branch";

    const mockExecSync = jest
      .spyOn(child_process, "execSync")
      .mockReturnValue("commit");

    const mockExec = jest
      .spyOn(child_process, "exec")
      .mockImplementation((command, options, callback) => {
        if (callback) {
          return callback(
            null,
            '{"packages":[],"tasks":[]}',
            "stderr"
          ) as unknown as ChildProcess;
        }
        return {} as unknown as ChildProcess;
      });

    turboIgnore(undefined, { directory: "__fixtures__/app" });

    validateLogs(
      [
        "Using Turborepo to determine if this project is affected by the commit...\n",
        'Inferred "test-app" as workspace from "package.json"',
        'Inferred turbo version ^2 based on "tasks" in "turbo.json"',
        'Using "build" as the task as it was unspecified',
        "Conflicting commit messages found: [vercel deploy] and [vercel skip]",
        `Found previous deployment ("last-deployed-sha") for "test-app" on branch "my-branch"`,
        'Analyzing results of `npx -y turbo@^2 run build --filter="test-app...[last-deployed-sha]" --dry=json`',
        "This project and its dependencies are not affected",
        () => expect.stringContaining("⏭ Ignoring the change"),
      ],
      mockConsole.log,
      { prefix: "≫  " }
    );

    expectIgnore(mockExit);
    mockExecSync.mockRestore();
    mockExec.mockRestore();
  });

  it("passes max buffer to turbo execution", () => {
    const mockExec = jest
      .spyOn(child_process, "exec")
      .mockImplementation((command, options, callback) => {
        if (callback) {
          return callback(
            null,
            '{"packages": [],"tasks":[]}',
            "stderr"
          ) as unknown as ChildProcess;
        }
        return {} as unknown as ChildProcess;
      });

    turboIgnore(undefined, { directory: "__fixtures__/app", maxBuffer: 1024 });

    expect(mockExec).toHaveBeenCalledWith(
      `npx -y turbo@^2 run build --filter="test-app...[HEAD^]" --dry=json`,
      expect.objectContaining({ maxBuffer: 1024 }),
      expect.anything()
    );

    mockExec.mockRestore();
  });

  it("runs with telemetry", () => {
    const mockExec = jest
      .spyOn(child_process, "exec")
      .mockImplementation((command, options, callback) => {
        if (callback) {
          return callback(
            null,
            '{"packages": [],"tasks":[]}',
            "stderr"
          ) as unknown as ChildProcess;
        }
        return {} as unknown as ChildProcess;
      });

    turboIgnore(undefined, {
      directory: "__fixtures__/app",
      maxBuffer: 1024,
      telemetry,
    });

    expect(mockExec).toHaveBeenCalledWith(
      `npx -y turbo@^2 run build --filter="test-app...[HEAD^]" --dry=json`,
      expect.objectContaining({ maxBuffer: 1024 }),
      expect.anything()
    );

    mockExec.mockRestore();
  });

  it("allows build if packages is missing", () => {
    const mockExec = jest
      .spyOn(child_process, "exec")
      .mockImplementation((command, options, callback) => {
        if (callback) {
          return callback(
            null,
            '{"tasks":[]}',
            "stderr"
          ) as unknown as ChildProcess;
        }
        return {} as unknown as ChildProcess;
      });

    turboIgnore(undefined, {
      directory: "__fixtures__/app",
    });

    expectBuild(mockExit);
    mockExec.mockRestore();
  });

  it("defaults to latest turbo if no hints for version", () => {
    const mockExec = jest
      .spyOn(child_process, "exec")
      .mockImplementation((command, options, callback) => {
        if (callback) {
          return callback(
            null,
            '{"packages": [],"tasks":[]}',
            "stderr"
          ) as unknown as ChildProcess;
        }
        return {} as unknown as ChildProcess;
      });

    turboIgnore(undefined, { directory: "__fixtures__/invalid_turbo_json" });

    expect(mockExec).toHaveBeenCalledWith(
      `npx -y turbo run build --filter="test-app...[HEAD^]" --dry=json`,
      expect.anything(),
      expect.anything()
    );

    mockExec.mockRestore();
  });
});

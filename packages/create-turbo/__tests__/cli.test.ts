import childProcess, { execSync, spawn } from "child_process";
import fs from "fs";
import path from "path";
import util from "util";
import semver from "semver";
import stripAnsi from "strip-ansi";
import { PackageManager, PACKAGE_MANAGERS } from "../src/constants";

const DEFAULT_APP_NAME = "my-turborepo";

const execFile = util.promisify(childProcess.execFile);

const keys = {
  up: "\x1B\x5B\x41",
  down: "\x1B\x5B\x42",
  enter: "\x0D",
  space: "\x20",
};

const createTurbo = path.resolve(__dirname, "../dist/index.js");
const cwd = path.join(__dirname, "../../../..");
const testDir = path.join(cwd, DEFAULT_APP_NAME);

// Increase default timeout for this test file
// since we are spawning a process to call turbo CLI and that can take some time
// This may be overriden in individual tests with a third param to the `it` block. E.g.:
// it('', () => {}, <override ms>)
jest.setTimeout(10_000);

const EXPECTED_HELP_MESSAGE = `
"
  Create a new Turborepo

  Usage:
    $ npx create-turbo [flags...] [<dir>]

  If <dir> is not provided up front you will be prompted for it.

  Flags:
    --use-npm           Explicitly tell the CLI to bootstrap the app using npm
    --use-pnpm          Explicitly tell the CLI to bootstrap the app using pnpm
    --use-yarn          Explicitly tell the CLI to bootstrap the app using yarn
    --no-install        Explicitly do not run the package manager's install command
    --help, -h          Show this help message
    --version, -v       Show the version of this script

"
`;

describe("create-turbo cli", () => {
  beforeAll(() => {
    cleanupTestDir();

    if (!fs.existsSync(createTurbo)) {
      // TODO: Consider running the build here instead of throwing
      throw new Error(
        `Cannot run Turbrepo CLI tests without building create-turbo`
      );
    }
  });

  afterAll(() => {
    // clean up after the whole test suite just in case any excptions prevented beforeEach callback from running
    cleanupTestDir();
  });

  beforeEach(() => {
    // cleanup before each test case in case the previous test timed out.
    cleanupTestDir();
  });

  afterEach(() => {
    // clean up test dir in between test cases since we are using the same directory for each one.
    cleanupTestDir();
  });

  describe("--no-install", () => {
    it(
      "default: guides the user through the process",
      async () => {
        configurePackageManager(PACKAGE_MANAGERS["npm"][0]);
        const cli = spawn("node", [createTurbo, "--no-install"], { cwd });

        const stdout = await runInteractiveCLI(cli);

        expect(stdout).toContain(
          ">>> Welcome to Turborepo! Let's get you set up with a new codebase."
        );

        expect(stdout).toContain(
          `? Where would you like to create your turborepo? (./${DEFAULT_APP_NAME})`
        );

        expect(stdout).toMatch(
          /\? Which package manager do you want to use\? \(Use arrow keys\)\n.*npm \n.*pnpm \n.*yarn \n/
        );

        expect(stdout).toContain(
          ">>> Created a new turborepo with the following:"
        );

        expect(stdout).toContain(
          `>>> Success! Created a new Turborepo at "${DEFAULT_APP_NAME}"`
        );

        expect(getGeneratedPackageJSON().packageManager).toMatch(/^npm/);

        expect(fs.existsSync(path.join(testDir, "node_modules"))).toBe(false);
      },
      1000 * 30
    );

    Object.values(PACKAGE_MANAGERS)
      .flat()
      .forEach((packageManager) => {
        it(
          `--use-${packageManager.command}: guides the user through the process (${packageManager.name})`,
          async () => {
            configurePackageManager(packageManager);
            const cli = spawn(
              "node",
              [createTurbo, "--no-install", `--use-${packageManager.command}`],
              { cwd }
            );

            const stdout = await runInteractiveCLI(cli);

            expect(stdout).toContain(
              ">>> Welcome to Turborepo! Let's get you set up with a new codebase."
            );

            expect(stdout).toContain(
              `? Where would you like to create your turborepo? (./${DEFAULT_APP_NAME})`
            );

            expect(stdout).toContain(
              ">>> Created a new turborepo with the following:"
            );

            expect(stdout).toContain(
              `>>> Success! Created a new Turborepo at "${DEFAULT_APP_NAME}"`
            );

            expect(getGeneratedPackageJSON().packageManager).toMatch(
              new RegExp(`^${packageManager.command}`)
            );

            expect(fs.existsSync(path.join(testDir, "node_modules"))).toBe(
              false
            );
          },
          1000 * 30
        );
      });
  });

  describe("with installation", () => {
    it(
      "default",
      async () => {
        configurePackageManager(PACKAGE_MANAGERS["npm"][0]);
        const cli = spawn("node", [createTurbo, `./${DEFAULT_APP_NAME}`], {
          cwd,
        });

        const stdout = await runInteractiveCLI(cli);

        expect(stdout).toContain(
          ">>> Welcome to Turborepo! Let's get you set up with a new codebase."
        );

        expect(stdout).toMatch(
          /\? Which package manager do you want to use\? \(Use arrow keys\)\n.*npm \n.*pnpm \n.*yarn \n/
        );

        expect(stdout).toContain(
          ">>> Created a new turborepo with the following:"
        );

        expect(stdout).toContain(
          `>>> Success! Created a new Turborepo at "${DEFAULT_APP_NAME}"`
        );

        expect(getGeneratedPackageJSON().packageManager).toMatch(/^npm/);

        expect(fs.existsSync(path.join(testDir, "node_modules"))).toBe(true);
      },
      1000 * 120
    );

    Object.values(PACKAGE_MANAGERS)
      .flat()
      .forEach((packageManager) => {
        it(
          `--use-${packageManager.command} (${packageManager.name})`,
          async () => {
            configurePackageManager(packageManager);
            const cli = spawn(
              "node",
              [
                createTurbo,
                `--use-${packageManager.command}`,
                `./${DEFAULT_APP_NAME}`,
              ],
              { cwd }
            );

            const stdout = await runInteractiveCLI(cli);

            expect(stdout).toContain(
              ">>> Welcome to Turborepo! Let's get you set up with a new codebase."
            );

            expect(stdout).toContain(
              ">>> Created a new turborepo with the following:"
            );

            expect(stdout).toContain(
              `>>> Success! Created a new Turborepo at "${DEFAULT_APP_NAME}"`
            );

            expect(getGeneratedPackageJSON().packageManager).toMatch(
              new RegExp(`^${packageManager.command}`)
            );

            expect(fs.existsSync(path.join(testDir, "node_modules"))).toBe(
              true
            );
          },
          1000 * 120
        );
      });
  });

  describe("printing version", () => {
    it("--version flag works", async () => {
      let { stdout } = await execFile("node", [createTurbo, "--version"]);
      expect(!!semver.valid(stdout.trim())).toBe(true);
    });

    it("-v flag works", async () => {
      let { stdout } = await execFile("node", [createTurbo, "-v"]);
      expect(!!semver.valid(stdout.trim())).toBe(true);
    });
  });

  describe("printing help message", () => {
    it("--help flag works", async () => {
      let { stdout } = await execFile("node", [createTurbo, "--help"]);
      expect(stdout).toMatchInlineSnapshot(EXPECTED_HELP_MESSAGE);
    });

    it("-h flag works", async () => {
      let { stdout } = await execFile("node", [createTurbo, "-h"]);
      expect(stdout).toMatchInlineSnapshot(EXPECTED_HELP_MESSAGE);
    });
  });
});

async function runInteractiveCLI(
  cli: childProcess.ChildProcessWithoutNullStreams
): Promise<string> {
  let accumulator = "";

  return new Promise((resolve, reject) => {
    cli.stdout.on("data", (data) => {
      accumulator += data.toString("utf8");

      if (accumulator.match(/Which package manager do you want to use/g)) {
        cli.stdin.write(keys.enter);
      }

      if (accumulator.match(/Where would you like to create your turborepo/g)) {
        cli.stdin.write(keys.enter);
      }
    });

    cli.on("exit", () => {
      // Insert newlines between inquirer prompt passes.
      accumulator = accumulator.replaceAll("\u001b[G?", "\u001b[G\n?");

      // Removes the ANSI escape sequences that would result in things being excluded in the accumulator.
      resolve(stripAnsi(accumulator));
    });

    cli.on("error", (e) => {
      reject(e);
    });
  });
}

function cleanupTestDir() {
  if (fs.existsSync(testDir)) {
    fs.rmSync(testDir, { recursive: true });
  }
}

function getGeneratedPackageJSON() {
  return JSON.parse(
    fs.readFileSync(path.join(testDir, "package.json")).toString()
  );
}

function configurePackageManager(packageManager: PackageManager) {
  execSync(
    `corepack prepare ${packageManager.command}@${packageManager.version} --activate`,
    { stdio: "ignore", cwd }
  );
}

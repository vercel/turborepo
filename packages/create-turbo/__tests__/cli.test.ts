import childProcess, { execSync, spawn } from "child_process";
import fs from "fs";
import path from "path";
import util from "util";
import semver from "semver";
import stripAnsi from "strip-ansi";
import { PackageManager, PACKAGE_MANAGERS } from "../src/constants";

const execFile = util.promisify(childProcess.execFile);

const keys = {
  up: "\x1B\x5B\x41",
  down: "\x1B\x5B\x42",
  enter: "\x0D",
  space: "\x20",
};

const createTurbo = path.resolve(__dirname, "../dist/index.js");
const cwd = path.join(__dirname, "../../../..");

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

  describe("--no-install", () => {
    it(
      "interactively configure",
      async () => {
        let packageManager = PACKAGE_MANAGERS["npm"][0];
        configurePackageManager(packageManager);
        let testDir = `my-${packageManager.name}-interactive-no-install-turborepo`;
        const cli = spawn("node", [createTurbo, "--no-install"], { cwd });

        const stdout = await runInteractiveCLI(cli, testDir);

        expect(stdout).toContain(
          ">>> Welcome to Turborepo! Let's get you set up with a new codebase."
        );

        expect(stdout).toContain(
          `? Where would you like to create your turborepo? (./my-turborepo)`
        );

        expect(stdout).toMatch(
          /\? Which package manager do you want to use\? \(Use arrow keys\)\n.*npm \n.*pnpm \n.*yarn \n/
        );

        expect(stdout).toContain(
          ">>> Created a new turborepo with the following:"
        );

        expect(stdout).toContain(
          `>>> Success! Created a new Turborepo at "${testDir}"`
        );

        expect(getGeneratedPackageJSON(testDir).packageManager).toMatch(/^npm/);

        expect(fs.existsSync(path.join(cwd, testDir, "node_modules"))).toBe(
          false
        );
      },
      1000 * 30
    );

    it.each(Object.values(PACKAGE_MANAGERS).flat())(
      `--use-$command: guides the user through the process ($name)`,
      async (packageManager) => {
        configurePackageManager(packageManager);
        let testDir = `my-${packageManager.name}-no-install-turborepo`;
        const cli = spawn(
          "node",
          [createTurbo, "--no-install", `--use-${packageManager.command}`],
          { cwd }
        );

        const stdout = await runInteractiveCLI(cli, testDir);

        expect(stdout).toContain(
          ">>> Welcome to Turborepo! Let's get you set up with a new codebase."
        );

        expect(stdout).toContain(
          `? Where would you like to create your turborepo? (./my-turborepo)`
        );

        expect(stdout).toContain(
          ">>> Created a new turborepo with the following:"
        );

        expect(stdout).toContain(
          `>>> Success! Created a new Turborepo at "${testDir}"`
        );

        expect(getGeneratedPackageJSON(testDir).packageManager).toMatch(
          new RegExp(`^${packageManager.command}`)
        );

        expect(fs.existsSync(path.join(cwd, testDir, "node_modules"))).toBe(
          false
        );
      },
      1000 * 30
    );
  });

  describe("with installation", () => {
    it(
      "interactively configure and install",
      async () => {
        let packageManager = PACKAGE_MANAGERS["npm"][0];
        configurePackageManager(packageManager);
        let testDir = `my-${packageManager.name}-interactive-install-turborepo`;
        const cli = spawn("node", [createTurbo], { cwd });

        const stdout = await runInteractiveCLI(cli, testDir);

        expect(stdout).toContain(
          ">>> Welcome to Turborepo! Let's get you set up with a new codebase."
        );

        expect(stdout).toContain(
          `? Where would you like to create your turborepo? (./my-turborepo)`
        );

        expect(stdout).toMatch(
          /\? Which package manager do you want to use\? \(Use arrow keys\)\n.*npm \n.*pnpm \n.*yarn \n/
        );

        expect(stdout).toContain(
          ">>> Created a new turborepo with the following:"
        );

        expect(stdout).toContain(
          `>>> Success! Created a new Turborepo at "${testDir}"`
        );

        expect(getGeneratedPackageJSON(testDir).packageManager).toMatch(/^npm/);

        expect(fs.existsSync(path.join(cwd, testDir, "node_modules"))).toBe(
          true
        );
      },
      1000 * 60 * 5
    );

    it.each(Object.values(PACKAGE_MANAGERS).flat())(
      `--use-$command ($name)`,
      async (packageManager) => {
        configurePackageManager(packageManager);
        let testDir = `my-${packageManager.name}-noninteractive-install-turborepo`;
        const cli = spawn(
          "node",
          [createTurbo, `--use-${packageManager.command}`, testDir],
          { cwd }
        );

        const stdout = await runInteractiveCLI(cli, testDir);

        expect(stdout).toContain(
          ">>> Welcome to Turborepo! Let's get you set up with a new codebase."
        );

        expect(stdout).toContain(
          ">>> Created a new turborepo with the following:"
        );

        expect(stdout).toContain(
          `>>> Success! Created a new Turborepo at "${testDir}"`
        );

        expect(getGeneratedPackageJSON(testDir).packageManager).toMatch(
          new RegExp(`^${packageManager.command}`)
        );

        expect(fs.existsSync(path.join(cwd, testDir, "node_modules"))).toBe(
          true
        );
      },
      1000 * 60 * 5
    );
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
  cli: childProcess.ChildProcessWithoutNullStreams,
  projectDirectory: string = ""
): Promise<string> {
  let accumulator = "";
  let advancedPackageManager = false;
  let advancedPath = false;

  return new Promise((resolve, reject) => {
    cli.stdout.on("data", (data) => {
      accumulator += data.toString("utf8");

      if (
        !advancedPackageManager &&
        accumulator.match(/Which package manager do you want to use/g)
      ) {
        advancedPackageManager = true;
        cli.stdin.write(keys.enter);
      }

      if (
        !advancedPath &&
        accumulator.match(/Where would you like to create your turborepo/g)
      ) {
        advancedPath = true;
        cli.stdin.write(Buffer.from(projectDirectory, "utf8"));
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

const testDirRegex = /^my-.+-turborepo$/;
function cleanupTestDir() {
  let children = fs.readdirSync(cwd);
  children.forEach((childDir) => {
    if (testDirRegex.test(childDir)) {
      fs.rmSync(path.join(cwd, childDir), { recursive: true });
    }
  });
}

function getGeneratedPackageJSON(testDir: string) {
  return JSON.parse(
    fs.readFileSync(path.join(cwd, testDir, "package.json")).toString()
  );
}

function configurePackageManager(packageManager: PackageManager) {
  if (packageManager.name === "npm") {
    // Corepack on Windows in GitHub CI is failing to progress on first invocation.
    // It's actually unnecessary to use Corepack for `npm` at this time, so skip it.
    execSync(`corepack disable`);
  } else {
    execSync(`corepack enable`);
    execSync(
      `corepack prepare ${packageManager.command}@${packageManager.version} --activate`,
      { stdio: "ignore", cwd }
    );
  }
}

import childProcess, { execSync } from "child_process";
import fs from "fs-extra";
import path from "path";
import util from "util";
import semver from "semver";
import stripAnsi from "strip-ansi";

const DEFAULT_APP_NAME = "my-turborepo";

const execFile = util.promisify(childProcess.execFile);
const spawn = childProcess.spawn;

const PACKAGE_MANAGERS = ["npm", "yarn", "pnpm"];

const keys = {
  up: "\x1B\x5B\x41",
  down: "\x1B\x5B\x42",
  enter: "\x0D",
  space: "\x20",
};

const createTurbo = path.resolve(__dirname, "../dist/index.js");
const testDir = path.join(__dirname, "../my-turborepo");

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
    execSync("corepack disable", { stdio: "ignore" });
    cleanupTestDir();

    if (fs.existsSync(testDir)) {
      fs.rmSync(testDir, { recursive: true });
    }

    if (!fs.existsSync(createTurbo)) {
      // TODO: Consider running the build here instead of throwing
      throw new Error(
        `Cannot run Turbrepo CLI tests without building create-turbo`
      );
    }
  });

  afterAll(() => {
    execSync("corepack enable", { stdio: "ignore" });

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
    it("default: guides the user through the process", async () => {
      const cli = spawn("node", [createTurbo, "--no-install"], {});

      const messages = await runInteractiveCLI(cli);

      expect(messages[0]).toEqual(
        ">>> Welcome to Turborepo! Let's get you set up with a new codebase."
      );

      expect(messages[1]).toEqual(
        `? Where would you like to create your turborepo? (./${DEFAULT_APP_NAME})`
      );

      expect(getPromptChoices(messages[2])).toEqual(["npm", "pnpm", "yarn"]);

      expect(messages[3]).toMatch(
        /^>>> Bootstrapped a new turborepo with the following:/
      );

      expect(
        messages.find((msg) =>
          msg.match(
            new RegExp(
              `>>> Success! Created a new Turborepo at "${DEFAULT_APP_NAME}"`
            )
          )
        )
      ).toBeTruthy();

      expect(getGeneratedPackageJSON().packageManager).toMatch(/^npm/);

      expect(fs.existsSync(path.join(testDir, "node_modules"))).toBe(false);
    });

    PACKAGE_MANAGERS.forEach((packageManager) => {
      it(`--use-${packageManager}: guides the user through the process`, async () => {
        const cli = spawn(
          "node",
          [createTurbo, "--no-install", `--use-${packageManager}`],
          {}
        );
        const messages = await runInteractiveCLI(cli);

        expect(messages[0]).toEqual(
          ">>> Welcome to Turborepo! Let's get you set up with a new codebase."
        );

        expect(messages[1]).toEqual(
          `? Where would you like to create your turborepo? (./${DEFAULT_APP_NAME})`
        );

        expect(messages[2]).toMatch(
          /^>>> Bootstrapped a new turborepo with the following:/
        );

        expect(
          messages.find((msg) =>
            msg.match(
              new RegExp(
                `>>> Success! Created a new Turborepo at "${DEFAULT_APP_NAME}"`
              )
            )
          )
        ).toBeTruthy();

        expect(getGeneratedPackageJSON().packageManager).toMatch(
          new RegExp(`^${packageManager}`)
        );

        expect(fs.existsSync(path.join(testDir, "node_modules"))).toBe(false);
      });
    });
  });

  describe("with installation", () => {
    it("default", async () => {
      const cli = spawn("node", [createTurbo, `./${DEFAULT_APP_NAME}`], {});

      const messages = await runInteractiveCLI(cli);

      expect(messages[0]).toEqual(
        ">>> Welcome to Turborepo! Let's get you set up with a new codebase."
      );

      expect(getPromptChoices(messages[1])).toEqual(["npm", "pnpm", "yarn"]);

      expect(messages[2]).toMatch(
        /^>>> Creating a new turborepo with the following:/
      );

      expect(
        messages.find((msg) =>
          msg.match(
            new RegExp(
              `>>> Success! Created a new Turborepo at "${DEFAULT_APP_NAME}"`
            )
          )
        )
      ).toBeTruthy();

      expect(getGeneratedPackageJSON().packageManager).toMatch(/^npm/);

      expect(fs.existsSync(path.join(testDir, "node_modules"))).toBe(true);
    }, 100_000);

    PACKAGE_MANAGERS.forEach((packageManager) => {
      it(`--use-${packageManager}`, async () => {
        const cli = spawn(
          "node",
          [createTurbo, `--use-${packageManager}`, `./${DEFAULT_APP_NAME}`],
          {}
        );
        const messages = await runInteractiveCLI(cli);

        expect(messages[0]).toEqual(
          ">>> Welcome to Turborepo! Let's get you set up with a new codebase."
        );

        expect(messages[1]).toMatch(
          /^>>> Creating a new turborepo with the following:/
        );

        expect(
          messages.find((msg) =>
            msg.match(
              new RegExp(
                `>>> Success! Created a new Turborepo at "${DEFAULT_APP_NAME}"`
              )
            )
          )
        ).toBeTruthy();

        expect(getGeneratedPackageJSON().packageManager).toMatch(
          new RegExp(`^${packageManager}`)
        );

        expect(fs.existsSync(path.join(testDir, "node_modules"))).toBe(true);
      }, 100_000);
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
): Promise<string[]> {
  return new Promise((resolve, reject) => {
    let previousPrompt: string;
    const messages: string[] = [];

    cli.stdout.on("data", (data) => {
      let prompt = cleanPrompt(data);

      if (
        !prompt ||
        prompt.startsWith(">>> TURBOREPO") ||
        isSamePrompt(prompt, previousPrompt)
      ) {
        return;
      }

      messages.push(prompt);

      if (prompt.match(/Which package manager do you want to use/)) {
        cli.stdin.write(keys.enter);
      }

      if (prompt.match(/Where would you like to create your turborepo/)) {
        cli.stdin.write(keys.enter);
      }

      previousPrompt = prompt;
    });

    cli.on("exit", () => {
      resolve(messages);
    });

    cli.on("error", (e) => {
      reject(e);
    });
  });
}

// These utils are a bit gnarly but they help me deal with the weirdness of node
// process stdout data formatting and inquirer. They're gross but make the tests
// easier to read than inlining everything IMO. Would be thrilled to delete them tho.
function cleanPrompt<T extends { toString(): string }>(data: T): string {
  return stripAnsi(data.toString())
    .trim()
    .split("\n")
    .map((s) => s.replace(/\s+$/, ""))
    .join("\n");
}

function getPromptChoices(prompt: string) {
  return prompt
    .slice(prompt.indexOf("❯") + 2)
    .split("\n")
    .map((s) => s.trim());
}

function isSamePrompt(
  currentPrompt: string,
  previousPrompt: string | undefined
) {
  if (previousPrompt === undefined) {
    return false;
  }
  let promptStart = previousPrompt.split("\n")[0];
  promptStart = promptStart.slice(0, promptStart.lastIndexOf("("));

  return currentPrompt.startsWith(promptStart);
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

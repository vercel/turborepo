import childProcess from "child_process";
import fs from "fs-extra";
import path from "path";
import util from "util";
import semver from "semver";
import stripAnsi from "strip-ansi";

const DEFAULT_APP_NAME = "my-turborepo";

const execFile = util.promisify(childProcess.execFile);
const spawn = childProcess.spawn;

const keys = {
  up: "\x1B\x5B\x41",
  down: "\x1B\x5B\x42",
  enter: "\x0D",
  space: "\x20",
};

const createTurbo = path.resolve(__dirname, "../dist/index.js");
const testDir = path.join(__dirname, "../my-turborepo");
const DEFAULT_JEST_TIMEOUT = 10000;

describe("create-turbo cli", () => {
  beforeAll(() => {
    jest.setTimeout(DEFAULT_JEST_TIMEOUT * 3);
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
    jest.setTimeout(DEFAULT_JEST_TIMEOUT);
    if (fs.existsSync(testDir)) {
      fs.rmSync(testDir, { recursive: true });
    }
  });

  it("guides the user through the process", (done) => {
    let cli = spawn("node", [createTurbo, "--no-install"], {});
    let promptCount = 0;
    let previousPrompt: string;
    const messages: string[] = [];
    cli.stdout.on("data", async (data) => {
      let prompt = cleanPrompt(data);

      if (
        !prompt ||
        prompt.startsWith(">>> TURBOREPO") ||
        isSamePrompt(prompt, previousPrompt)
      ) {
        return;
      }

      promptCount++;

      switch (promptCount) {
        case 1:
          expect(prompt).toEqual(
            ">>> Welcome to Turborepo! Let's get you set up with a new codebase."
          );
          break;
        case 2:
          expect(prompt).toEqual(
            `? Where would you like to create your turborepo? (./${DEFAULT_APP_NAME})`
          );
          cli.stdin.write(keys.enter);
          break;

        case 3:
          // Which package manager do you want to use?
          // easy to change deployment targets.
          expect(getPromptChoices(prompt)).toEqual(["npm", "pnpm", "yarn"]);
          cli.stdin.write(keys.enter);
          break;
        case 4:
          // Bootstrap info
          expect(
            prompt.startsWith(
              ">>> Bootstrapped a new turborepo with the following:"
            )
          ).toBe(true);

          break;
      }

      previousPrompt = prompt;
    });

    cli.on("exit", () => {
      try {
        done();
      } catch (error) {
        done(error);
      }
      return;
    });
  }, 10000);

  describe("the --version flag", () => {
    it("prints the current version", async () => {
      let { stdout } = await execFile("node", [createTurbo, "--version"]);
      expect(!!semver.valid(stdout.trim())).toBe(true);
    });
  });

  describe("the -v flag", () => {
    it("prints the current version", async () => {
      let { stdout } = await execFile("node", [createTurbo, "-v"]);
      expect(!!semver.valid(stdout.trim())).toBe(true);
    });
  });

  describe("the --help flag", () => {
    it("prints help info", async () => {
      let { stdout } = await execFile("node", [createTurbo, "--help"]);

      expect(stdout).toMatchInlineSnapshot(`
        "
          Create a new Turborepo

          Usage:
            $ npx create-turbo [flags...] [<dir>]

          If <dir> is not provided up front you will be prompted for it.

          Flags:
            --use-npm           Explicitly tell the CLI to bootstrap the app using npm
            --use-pnpm          Explicitly tell the CLI to bootstrap the app using pnpm
            --no-install        Explicitly do not run the package manager's install command
            --help, -h          Show this help message
            --version, -v       Show the version of this script

        "
      `);
    });
  });

  describe("the -h flag", () => {
    it("prints help info", async () => {
      let { stdout } = await execFile("node", [createTurbo, "-h"]);
      expect(stdout).toMatchInlineSnapshot(`
        "
          Create a new Turborepo

          Usage:
            $ npx create-turbo [flags...] [<dir>]

          If <dir> is not provided up front you will be prompted for it.

          Flags:
            --use-npm           Explicitly tell the CLI to bootstrap the app using npm
            --use-pnpm          Explicitly tell the CLI to bootstrap the app using pnpm
            --no-install        Explicitly do not run the package manager's install command
            --help, -h          Show this help message
            --version, -v       Show the version of this script

        "
      `);
    });
  });
});

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

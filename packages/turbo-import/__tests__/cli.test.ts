import childProcess from "child_process";
import fs from "fs-extra";
import path from "path";
import util from "util";
import semver from "semver";
import stripAnsi from "strip-ansi";

const DEFAULT_APP_NAME = "my-turborepo";

const execFile = util.promisify(childProcess.execFile);
const spawn = childProcess.spawn;

const turboImport = path.resolve(__dirname, "../dist/index.js");
const testDir = path.join(__dirname, "../my-turborepo");
const fixture = path.join(__dirname, "../__fixtures__");
const DEFAULT_JEST_TIMEOUT = 10000;

describe("@turbo/import cli", () => {
  beforeAll(() => {
    jest.setTimeout(DEFAULT_JEST_TIMEOUT * 3);
    if (fs.existsSync(testDir)) {
      fs.rmSync(testDir, { recursive: true });
    }

    if (!fs.existsSync(turboImport)) {
      // TODO: Consider running the build here instead of throwing
      throw new Error(
        `Cannot run Turbrepo CLI tests without building @turbo/import`
      );
    }
  });

  afterAll(() => {
    jest.setTimeout(DEFAULT_JEST_TIMEOUT);
    // if (fs.existsSync(testDir)) {
    //   fs.rmSync(testDir, { recursive: true });
    // }
  });

  it("guides the user through the process", (done) => {
    fs.copySync(path.join(fixture, "remix"), testDir);

    let cli = spawn("node", [turboImport, "--no-install"], {
      cwd: testDir,
    });
    let promptCount = 0;
    let previousPrompt: string;

    cli.on("exit", () => {
      try {
        done();
      } catch (error) {
        done(error);
      }
      return;
    });
  }, 10000);
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

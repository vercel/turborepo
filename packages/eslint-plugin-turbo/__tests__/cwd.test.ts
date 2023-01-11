import fs from "fs";
import path from "path";
import JSON5 from "json5";
import { execSync } from "child_process";

describe("eslint settings check", () => {
  beforeAll(() => {
    const cwd = path.join(__dirname, "fixtures", "workspace");
    execSync(`npm install`, { cwd });
  });

  afterAll(() => {
    const nodeModulesDir = path.join(
      __dirname,
      "fixtures",
      "workspace",
      "node_modules"
    );
    fs.rmSync(nodeModulesDir, { force: true, recursive: true });
  });

  it("does the right thing for peers", () => {
    const cwd = path.join(__dirname, "fixtures", "workspace");
    const configString = execSync(`eslint --print-config peer.js`, {
      cwd,
      encoding: "utf8",
    });
    const configJson = JSON.parse(configString);

    expect(configJson.settings).toEqual({
      turbo: { envVars: ["CI", "UNORDERED"] },
    });
  });

  it("does the right thing for child dirs", () => {
    const cwd = path.join(__dirname, "fixtures", "workspace", "child");
    const configString = execSync(`eslint --print-config child.js`, {
      cwd,
      encoding: "utf8",
    });
    const configJson = JSON.parse(configString);

    expect(configJson.settings).toEqual({
      turbo: { envVars: ["CI", "UNORDERED"] },
    });
  });
});

describe("eslint cache is busted", () => {
  let turboJsonPath: string;
  let originalString: string;

  beforeAll(() => {
    const cwd = path.join(__dirname, "fixtures", "workspace");
    execSync(`npm install`, { cwd });

    turboJsonPath = path.join(__dirname, "fixtures", "workspace", "turbo.json");
    originalString = fs.readFileSync(turboJsonPath, { encoding: "utf8" });
  });

  afterEach(() => {
    fs.writeFileSync(turboJsonPath, originalString);
  });

  afterAll(() => {
    fs.writeFileSync(turboJsonPath, originalString);

    const nodeModulesDir = path.join(
      __dirname,
      "fixtures",
      "workspace",
      "node_modules"
    );
    fs.rmSync(nodeModulesDir, { force: true, recursive: true });
  });

  it("catches a lint error after changing config", () => {
    expect.assertions(2);

    // ensure that we populate the cache with a failure.
    const cwd = path.join(__dirname, "fixtures", "workspace", "child");
    try {
      execSync(`eslint --format=json child.js`, { cwd, encoding: "utf8" });
    } catch (error: any) {
      const outputJson = JSON.parse(error.stdout);
      expect(outputJson).toMatchObject([
        {
          messages: [
            {
              message:
                "$NONEXISTENT is not listed as a dependency in turbo.json",
            },
          ],
        },
      ]);
    }

    // change the configuration
    const turboJson = JSON5.parse(originalString);
    turboJson.globalEnv = ["CI", "NONEXISTENT"];
    fs.writeFileSync(turboJsonPath, JSON.stringify(turboJson, null, 2));

    // test that we invalidated the eslint cache
    const output = execSync(`eslint --format=json child.js`, {
      cwd,
      encoding: "utf8",
    });
    const outputJson = JSON.parse(output);
    expect(outputJson).toMatchObject([{ errorCount: 0 }]);
  });
});

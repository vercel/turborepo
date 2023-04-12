import path from "path";
import JSON5 from "json5";
import { execSync } from "child_process";
import { Schema } from "@turbo/types";
import { setupTestFixtures } from "@turbo/test-utils";

describe("eslint settings check", () => {
  const { useFixture } = setupTestFixtures({
    directory: path.join(__dirname, "../"),
  });

  it("does the right thing for peers", () => {
    const { root: cwd } = useFixture({ fixture: "workspace" });
    execSync(`npm install`, { cwd });

    const configString = execSync(`eslint --print-config peer.js`, {
      cwd,
      encoding: "utf8",
    });
    const configJson = JSON5.parse(configString);

    expect(configJson.settings).toEqual({
      turbo: { envVars: ["CI", "UNORDERED"] },
    });
  });

  it("does the right thing for child dirs", () => {
    const { root } = useFixture({ fixture: "workspace" });
    execSync(`npm install`, { cwd: root });

    const cwd = path.join(root, "child");
    const configString = execSync(`eslint --print-config child.js`, {
      cwd,
      encoding: "utf8",
    });
    const configJson = JSON5.parse(configString);

    expect(configJson.settings).toEqual({
      turbo: { envVars: ["CI", "UNORDERED"] },
    });
  });
});

describe("eslint cache is busted", () => {
  const { useFixture } = setupTestFixtures({
    directory: path.join(__dirname, "../"),
  });

  it("catches a lint error after changing config", () => {
    expect.assertions(2);

    // ensure that we populate the cache with a failure.
    const { root, readJson, write } = useFixture({ fixture: "workspace" });
    execSync(`npm install`, { cwd: root });

    const cwd = path.join(root, "child");
    try {
      execSync(`eslint --format=json child.js`, { cwd, encoding: "utf8" });
    } catch (error: any) {
      const outputJson = JSON5.parse(error.stdout);
      expect(outputJson).toMatchObject([
        {
          messages: [
            {
              message:
                "NONEXISTENT is not listed as a dependency in turbo.json",
            },
          ],
        },
      ]);
    }

    // change the configuration
    const turboJson = readJson<Schema>("turbo.json");
    if (turboJson && "globalEnv" in turboJson) {
      turboJson.globalEnv = ["CI", "NONEXISTENT"];
      write("turbo.json", JSON5.stringify(turboJson, null, 2));
    }

    // test that we invalidated the eslint cache
    const output = execSync(`eslint --format=json child.js`, {
      cwd,
      encoding: "utf8",
    });
    const outputJson = JSON5.parse(output);
    expect(outputJson).toMatchObject([{ errorCount: 0 }]);
  });
});

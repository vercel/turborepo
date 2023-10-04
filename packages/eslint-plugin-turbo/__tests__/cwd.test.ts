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

    const configString = execSync(`npm exec eslint -- --print-config peer.js`, {
      cwd,
      encoding: "utf8",
    });
    const configJson = JSON5.parse(configString);

    expect(configJson.settings).toEqual({
      turbo: {
        cacheKey: {
          global: {
            legacyConfig: [],
            env: ["CI", "UNORDERED"],
            passThroughEnv: null,
            dotEnv: {
              filePaths: [".env", "missing.env"],
              hashes: {
                ".env": "9ad6c5fd4d5bbe7c00e1f2b358ac7ef2aa3521d0",
              },
            },
          },
          globalTasks: {
            build: {
              legacyConfig: [],
              env: [],
              passThroughEnv: null,
              dotEnv: null,
            },
            test: {
              legacyConfig: [],
              env: [],
              passThroughEnv: null,
              dotEnv: null,
            },
            lint: {
              legacyConfig: [],
              env: [],
              passThroughEnv: null,
              dotEnv: null,
            },
            deploy: {
              legacyConfig: [],
              env: [],
              passThroughEnv: null,
              dotEnv: null,
            },
          },
          workspaceTasks: {},
        },
      },
    });
  });

  it("does the right thing for child dirs", () => {
    const { root } = useFixture({ fixture: "workspace" });
    execSync(`npm install`, { cwd: root });

    const cwd = path.join(root, "child");
    const configString = execSync(
      `npm exec eslint -- --print-config child.js`,
      {
        cwd,
        encoding: "utf8",
      }
    );
    const configJson = JSON5.parse(configString);

    expect(configJson.settings).toEqual({
      turbo: {
        cacheKey: {
          global: {
            legacyConfig: [],
            env: ["CI", "UNORDERED"],
            passThroughEnv: null,
            dotEnv: {
              filePaths: [".env", "missing.env"],
              hashes: {
                ".env": "9ad6c5fd4d5bbe7c00e1f2b358ac7ef2aa3521d0",
              },
            },
          },
          globalTasks: {
            build: {
              legacyConfig: [],
              env: [],
              passThroughEnv: null,
              dotEnv: null,
            },
            test: {
              legacyConfig: [],
              env: [],
              passThroughEnv: null,
              dotEnv: null,
            },
            lint: {
              legacyConfig: [],
              env: [],
              passThroughEnv: null,
              dotEnv: null,
            },
            deploy: {
              legacyConfig: [],
              env: [],
              passThroughEnv: null,
              dotEnv: null,
            },
          },
          workspaceTasks: {},
        },
      },
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
      execSync(`npm exec eslint -- --format=json child.js`, {
        cwd,
        encoding: "utf8",
      });
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
    const output = execSync(`npm exec eslint -- --format=json child.js`, {
      cwd,
      encoding: "utf8",
    });
    const outputJson = JSON5.parse(output);
    expect(outputJson).toMatchObject([{ errorCount: 0 }]);
  });
});

import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

function changeJson(cwd, file, change) {
  const path = join(cwd, file);
  const json = JSON.parse(readFileSync(path, "utf8"));
  change(json);
  writeFileSync(path, `${JSON.stringify(json, null, 2)}\n`);
}

function fix(cwd) {
  changeJson(cwd, "turbo.json", (config) => {
    config.futureFlags = {
      ...config.futureFlags,
      strictTaskEntrypointSelection: true
    };
  });
}

export default {
  name: "task-entrypoints",
  fix,
  nearMisses: {
    "remove the test build dependency"(cwd) {
      fix(cwd);
      changeJson(cwd, "turbo.json", (config) => {
        config.tasks.test.dependsOn = [];
      });
    },
    "remove the transitive UI build dependency"(cwd) {
      fix(cwd);
      changeJson(cwd, "turbo.json", (config) => {
        config.tasks.build.dependsOn = [];
      });
    },
    "override only the docs test dependencies"(cwd) {
      mkdirSync(join(cwd, "apps/docs"), { recursive: true });
      writeFileSync(
        join(cwd, "apps/docs/turbo.json"),
        JSON.stringify(
          {
            extends: ["//"],
            tasks: { test: { dependsOn: [] } }
          },
          null,
          2
        ) + "\n"
      );
    },
    "delete the docs build script"(cwd) {
      fix(cwd);
      changeJson(cwd, "apps/docs/package.json", (config) => {
        delete config.scripts.build;
      });
    }
  }
};

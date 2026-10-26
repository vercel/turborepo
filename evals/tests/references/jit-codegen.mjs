import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

function changeConfig(cwd, change) {
  const file = join(cwd, "turbo.json");
  const config = JSON.parse(readFileSync(file, "utf8"));
  change(config);
  writeFileSync(file, `${JSON.stringify(config, null, 2)}\n`);
}

const sourceInputs = [
  "$TURBO_DEFAULT$",
  "!src/generated/**",
  "!dist/**",
  "!.turbo/**"
];
const generatedInput = { mode: "jit", globs: ["src/generated/**"] };

function fix(cwd) {
  changeConfig(cwd, (config) => {
    config.tasks.codegen.inputs = [...sourceInputs];
    config.tasks.build.inputs = [...sourceInputs, generatedInput];
  });
}

export default {
  name: "jit-codegen",
  fix,
  nearMisses: {
    "force rebuilds through the root build command"(cwd) {
      fix(cwd);
      const path = join(cwd, "package.json");
      const manifest = JSON.parse(readFileSync(path, "utf8"));
      manifest.scripts.build = "turbo run build --force";
      writeFileSync(path, `${JSON.stringify(manifest, null, 2)}\n`);
    },
    "defer only build inputs but leave codegen hashing old generated files"(
      cwd
    ) {
      changeConfig(cwd, (config) => {
        config.tasks.build.inputs = [...sourceInputs, generatedInput];
      });
    },
    "disable build caching instead of correcting its inputs"(cwd) {
      changeConfig(cwd, (config) => {
        config.tasks.build.cache = false;
      });
    },
    "exclude generated files without hashing the fresh result"(cwd) {
      changeConfig(cwd, (config) => {
        config.tasks.codegen.inputs = [...sourceInputs];
        config.tasks.build.inputs = [...sourceInputs];
      });
    },
    "hash the generated schema but omit renderer source from both tasks"(cwd) {
      changeConfig(cwd, (config) => {
        config.tasks.codegen.inputs = ["codegen.cjs"];
        config.tasks.build.inputs = ["build.cjs", generatedInput];
      });
    },
    "invalidate every workspace when web renderer changes"(cwd) {
      fix(cwd);
      changeConfig(cwd, (config) => {
        config.globalDependencies = ["apps/web/src/render.cjs"];
      });
    }
  }
};

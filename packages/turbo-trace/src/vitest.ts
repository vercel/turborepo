import { createRequire } from "node:module";
import { join, resolve } from "node:path";
import { inRepository, loadSelection, report } from "./selection";
import type { Selection } from "./selection";

interface TestConfig {
  dir?: string;
  exclude?: string[];
  passWithNoTests?: boolean;
  root?: string;
}

/** The dropped test files below `dir`, as patterns for `test.exclude`. */
function drop(selection: Selection, dir: string, exclude: string[]): void {
  const inside = inRepository(selection, dir);
  const prefix = inside === "" ? "" : `${inside}/`;
  const below = (file: string) => file.startsWith(prefix);
  const dropped = [...selection.dropped].filter(below);
  const total = selection.known.filter(below).length;
  exclude.push(
    ...dropped.map((file) =>
      file.slice(prefix.length).replace(/[\\()[\]{}!*?@+|]/g, "\\$&")
    )
  );
  report(selection, "vitest", dir, total - dropped.length, total);
}

/**
 * A Vite plugin that makes vitest run only the test files of the selection
 * named by `TURBO_TRACE_SELECTION`. It does nothing without the variable, and
 * nothing outside vitest.
 */
export function turboTrace() {
  const selection = () => (process.env.VITEST ? loadSelection() : undefined);
  let done = false;
  return {
    name: "turbo-trace",
    config(config: { root?: string; test?: TestConfig }) {
      const active = selection();
      if (!active) {
        return;
      }
      const test = (config.test ??= {});
      test.passWithNoTests = true;
      const root = resolve(test.root ?? config.root ?? "");
      try {
        // Setting `exclude` replaces vitest's defaults, and they differ
        // between versions: they are taken from the vitest of this project.
        test.exclude ??= [
          ...createRequire(join(root, "package.json"))("vitest/config")
            .configDefaults.exclude
        ];
      } catch {
        // Not found (vite-plus brings its own vitest): `configureVitest`.
        return;
      }
      drop(active, resolve(root, test.dir ?? ""), test.exclude);
      done = true;
    },
    // Vitest 3.1 and later: the resolved configuration, with the defaults.
    configureVitest({
      project
    }: {
      project: { config: Required<TestConfig> };
    }) {
      const active = selection();
      if (active && !done) {
        const { dir, exclude, root } = project.config;
        drop(active, dir || root, exclude);
      }
    }
  };
}

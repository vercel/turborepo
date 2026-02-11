import fs from "node:fs";
import Module from "node:module";
import { transform } from "sucrase";

let registered = false;

/**
 * Registers a require() hook that compiles TypeScript files on the fly using
 * sucrase.  This replaces the previous tsx/cjs/api dependency so that the
 * entire tool can be compiled into a single binary with no external runtime
 * dependencies.
 *
 * The hook is idempotent — calling it more than once is a no-op.
 */
export function registerTypeScript(): void {
  if (registered) {
    return;
  }
  registered = true;

  const extensions = (
    Module as unknown as { _extensions: NodeJS.RequireExtensions }
  )._extensions;

  const compile = (module: NodeJS.Module, filename: string): void => {
    const code = fs.readFileSync(filename, "utf-8");
    const result = transform(code, {
      transforms: ["typescript", "imports"],
      filePath: filename
    });
    (
      module as NodeJS.Module & {
        _compile: (code: string, filename: string) => void;
      }
    )._compile(result.code, filename);
  };

  extensions[".ts"] = compile;
  extensions[".tsx"] = compile;
}

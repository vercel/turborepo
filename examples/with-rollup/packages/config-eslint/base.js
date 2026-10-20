import { createRequire } from "node:module";

import babelParser from "@babel/eslint-parser";
import js from "@eslint/js";
import eslintConfigPrettier from "eslint-config-prettier";
import turboConfig from "eslint-config-turbo/flat";

const require = createRequire(import.meta.url);

const parser = {
  ...babelParser,
  parseForESLint(code, options) {
    const result = babelParser.parseForESLint(code, options);
    const { globalScope } = result.scopeManager;

    result.scopeManager.addGlobals = (names) => {
      for (const name of names) {
        globalScope.__defineGeneric(
          name,
          globalScope.set,
          globalScope.variables,
          null,
          null,
        );
      }

      const namesSet = new Set(names);
      globalScope.through = globalScope.through.filter((reference) => {
        const name = reference.identifier.name;
        if (!namesSet.has(name)) return true;

        const variable = globalScope.set.get(name);
        reference.resolved = variable;
        variable.references.push(reference);
        return false;
      });
      globalScope.implicit.variables = globalScope.implicit.variables.filter(
        (variable) => {
          if (!namesSet.has(variable.name)) return true;
          globalScope.implicit.set.delete(variable.name);
          return false;
        },
      );
      globalScope.implicit.left = globalScope.implicit.left.filter(
        (reference) => !namesSet.has(reference.identifier.name),
      );
    };

    return result;
  },
};

/**
 * A shared ESLint configuration for the repository.
 *
 * TypeScript is parsed with Babel instead of typescript-eslint because
 * typescript-eslint requires the legacy TypeScript compiler API, which the
 * native TypeScript 7 compiler no longer provides. The parser adapter adds the
 * current ESLint scope-manager API while Babel updates its parser integration.
 *
 * @type {import("eslint").Linter.Config[]}
 * */
export const config = [
  js.configs.recommended,
  ...turboConfig,
  eslintConfigPrettier,
  {
    files: ["**/*.ts", "**/*.tsx"],
    languageOptions: {
      parser,
      parserOptions: {
        requireConfigFile: false,
        babelOptions: {
          plugins: [require.resolve("@babel/plugin-syntax-jsx")],
          presets: [require.resolve("@babel/preset-typescript")],
        },
      },
    },
  },
  {
    ignores: ["dist/**", ".next/**", "coverage/**"],
  },
];

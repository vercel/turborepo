import { existsSync } from 'node:fs';
import path from 'node:path';
import prettier from 'eslint-config-prettier';
import { includeIgnoreFile } from '@eslint/config-helpers';
import js from '@eslint/js';
import svelte from 'eslint-plugin-svelte';
import globals from 'globals';
import tseslint from 'typescript-eslint';

/**
 * Traverses up directories to find the nearest .gitignore file.
 * @param {string} startDir - The path to start searching from.
 * @param {number} maxDepth - How many parent directories to check.
 * @returns {string|null} The resolved absolute path to .gitignore, or null if not found.
 */
function findGitignore(startDir, maxDepth = 2) {
  let currentDir = path.resolve(startDir);

  for (let depth = 0; depth <= maxDepth; depth++) {
    const potentialPath = path.join(currentDir, '.gitignore');

    if (existsSync(potentialPath)) {
      return potentialPath;
    }

    const parentDir = path.dirname(currentDir);
    // Stop if we reach the system root directory
    if (parentDir === currentDir) {
      break;
    }
    currentDir = parentDir;
  }

  return null;
}

/**
 * Creates a type-safe ESLint configuration tailored for Svelte 5 Monorepos.
 * @param {string} packageDir - The directory of the workspace executing the configuration.
 * @param {number} [maxIgnoreDepth=2] - Maximum parent directory depth to search for a .gitignore.
 */
export function createConfig(packageDir, maxIgnoreDepth = 2) {
  const gitignorePath = findGitignore(packageDir, maxIgnoreDepth);

  return tseslint.config(
    ...(gitignorePath ? [includeIgnoreFile(gitignorePath)] : []),
    js.configs.recommended,
    ...tseslint.configs.recommended,
    ...svelte.configs.recommended,
    prettier,
    svelte.configs.prettier,
    {
      languageOptions: {
        globals: { ...globals.browser, ...globals.node }
      },
      rules: {
        // typescript-eslint strongly recommend that you do not use the no-undef lint rule on TypeScript projects.
        // see: https://typescript-eslint.io/troubleshooting/faqs/eslint/#i-get-errors-from-the-no-undef-rule-about-global-variables-not-being-defined-even-though-there-are-no-typescript-errors
        'no-undef': 'off',
        // SVELTE 5 OPTIMIZATION: Prevent prefer-const from breaking bindable runes
        'prefer-const': 'warn'
      }
    },
    {
      files: ['**/*.svelte', '**/*.svelte.ts', '**/*.svelte.js'],
      languageOptions: {
        parserOptions: {
          extraFileExtensions: ['.svelte', '.svelte.ts'],
          parser: tseslint.parser
        }
      }
    }
  );
}

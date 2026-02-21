import ts from 'typescript-eslint';
import { config } from '@repo/eslint-config/index.js';

export default [
  ...config,
  {
    files: ['**/*.svelte', '**/*.svelte.ts', '**/*.ts'],
    ignores: ['.svelte-kit/**', 'dist/**'],
    languageOptions: {
      parserOptions: {
        parser: ts.parser
      }
    }
  }
];

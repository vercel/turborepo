import * as js from '@eslint/js';
import * as solid from 'eslint-plugin-solid';
import * as globals from 'globals';
import ts from 'typescript-eslint';

export const config = ts.config(
  js.configs.recommended,
  ...ts.configs.recommended,
  {
    ...solid,
    files: ["**/*.{ts,tsx}"],
    languageOptions: {
      globals: {
        ...globals.browser,
        ...globals.node
      }
    }
  }
);

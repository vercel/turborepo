const { resolve } = require('node:path');

const project = resolve(process.cwd(), 'tsconfig.json');

/**
 * This is a custom ESLint configuration for use with
 * Angular libraries
 */

/** @type {import("eslint").Linter.Config} */
module.exports = {
  extends: ['eslint:recommended', 'prettier', 'eslint-config-turbo'],
  plugins: ['only-warn'],
  parser: '@typescript-eslint/parser',
  parserOptions: {
    sourceType: 'module',
    ecmaVersion: 'latest'
  },
  settings: {
    'import/resolver': {
      typescript: {
        project
      }
    }
  },
  env: {
    browser: true,
    node: true
  },
  ignorePatterns: ['node_modules', 'dist/', 'projects/**/*'],
  overrides: [
    {
      files: ['*.ts'],
      extends: [
        'eslint:recommended',
        'plugin:@typescript-eslint/recommended',
        'plugin:@angular-eslint/recommended',
        'plugin:@angular-eslint/template/process-inline-templates'
      ],
      rules: {
        '@angular-eslint/directive-selector': [
          'error',
          {
            type: 'attribute',
            prefix: 'app',
            style: 'camelCase'
          }
        ],
        '@angular-eslint/component-selector': [
          'error',
          {
            type: 'element',
            prefix: 'app',
            style: 'kebab-case'
          }
        ]
      }
    },
    {
      files: ['*.html'],
      extends: [
        'plugin:@angular-eslint/template/recommended',
        'plugin:@angular-eslint/template/accessibility'
      ],
      rules: {}
    }
  ]
};

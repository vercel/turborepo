const { resolve } = require('node:path');

const project = resolve(process.cwd(), 'tsconfig.json');

/**
 * This is a custom ESLint configuration for use with
 * internal (bundled by their consumer) libraries
 * that utilize React.
 *
 * This config extends the Vercel Engineering Style Guide.
 * For more information, see https://github.com/vercel/style-guide
 *
 * @type {import("eslint").Linter.Config}
 */
module.exports = {
  extends: ['./base.js'],
  plugins: ['only-warn'],
  globals: {
    React: true,
    JSX: true,
  },
  env: {
    browser: true,
  },
  settings: {
    'import/resolver': {
      typescript: {
        project,
      },
    },
  },
  ignorePatterns: ['.*.js', 'node_modules/', 'dist/'],
  overrides: [{ files: ['*.js?(x)', '*.ts?(x)'] }],
};

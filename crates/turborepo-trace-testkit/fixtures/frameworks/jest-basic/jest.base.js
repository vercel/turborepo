const path = require('path');

module.exports = {
  transform: {
    '^.+\\.(t|j)sx?$': '@swc/jest',
    '^.+\\.css$': ['<rootDir>/test-utils/css-transform.js', { modules: true }],
  },
  moduleNameMapper: {
    // Regex groups, <rootDir> and an absolute path built from __dirname.
    '^@app/(.*)/(.*)$': '<rootDir>/src/$1/$2',
    '^@utils$': path.resolve(__dirname, 'test-utils', 'index.ts'),
    '^(\\.{1,2}/.*)\\.js$': '$1',
  },
  setupFilesAfterEnv: [],
};

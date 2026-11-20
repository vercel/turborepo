// A package's jest config on top of the repository's shared one. One mapper target is a
// sibling of a dependency inside the package manager's store, reached through the real
// path of the dependency's link in node_modules.
const path = require('path');
const rootConfig = require('./jest.base.js');

module.exports = {
  ...rootConfig,
  moduleNameMapper: {
    ...rootConfig.moduleNameMapper,
    '^@scope/shared$': path.resolve(
      require('fs').realpathSync(
        path.resolve(__dirname, 'node_modules/@scope/x'),
      ),
      '../shared/dist/index.js',
    ),
  },
  setupFilesAfterEnv: [
    ...(rootConfig.setupFilesAfterEnv ?? []),
    path.resolve(__dirname, 'jest-setup.ts'),
  ],
};

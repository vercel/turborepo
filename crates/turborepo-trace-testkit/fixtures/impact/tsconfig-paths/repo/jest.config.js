// moduleNameMapper derived from tsconfig paths (as ts-jest's pathsToModuleNameMapper does).
const tsconfig = require('./tsconfig.json');

const moduleNameMapper = {};
for (const [k, [v]] of Object.entries(tsconfig.compilerOptions.paths)) {
  moduleNameMapper[`^${k.replace('*', '(.*)')}$`] = `<rootDir>/${v.replace('*', '$1')}`;
}

/** @type {import('jest').Config} */
module.exports = {
  testEnvironment: 'node',
  transform: { '^.+\\.tsx?$': ['ts-jest', { isolatedModules: true }] },
  moduleNameMapper,
};

// A testMatch that is read exactly and matches no file, while jest's default patterns
// would match spec/s.test.js: the project has no tests (jest finds none either).
module.exports = { testMatch: ['<rootDir>/e2e/**/*.e2e.js'] };

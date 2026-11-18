module.exports = {
  projects: [
    '<rootDir>/packages/*',
    {
      displayName: 'tools',
      rootDir: '<rootDir>/tools',
      testMatch: ['<rootDir>/**/*.check.js'],
      testEnvironment: 'node',
    },
  ],
};

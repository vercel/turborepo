// Wraps next's SWC jest transformer: every test of the project is compiled through this file.
const next = require('next/dist/build/swc/jest-transformer');

module.exports = {
  createTransformer(options) {
    return next.createTransformer(options);
  },
};

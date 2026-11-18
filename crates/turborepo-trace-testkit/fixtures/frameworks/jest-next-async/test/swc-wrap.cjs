// Hands every file of the project to next's SWC transformer.
const swc = require('next/dist/build/swc/jest-transformer');

module.exports = {
  createTransformer: (opts) => swc.createTransformer(opts),
};

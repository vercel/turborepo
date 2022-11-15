// Our integration tests are setup to use the project root's index.js,
// but next-dev uses src/index.js. Just paper over this by having both.

export * from './src/index.js';
export { default } from './src/index.js';

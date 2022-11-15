// Our integration tests are setup to use the project root's index.js,
// but next-dev uses pages/index.js. Just paper over this by having both.

export * from "./pages/index.js";
export { default } from "./pages/index.js";

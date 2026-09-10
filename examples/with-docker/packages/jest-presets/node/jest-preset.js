/** @type {import("jest").Config} */
module.exports = {
  roots: ["<rootDir>"],
  transform: {
    "^.+\\.tsx?$": require.resolve("./esbuild-transformer"),
  },
  moduleFileExtensions: ["ts", "tsx", "js", "jsx", "json", "node"],
  modulePathIgnorePatterns: [
    "<rootDir>/test/__fixtures__",
    "<rootDir>/node_modules",
    "<rootDir>/dist",
  ],
};

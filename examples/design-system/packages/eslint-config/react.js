import globals from "globals";
import library from "./library.js";

export default [
  ...library,
  {
    languageOptions: {
      globals: globals.browser,
      parserOptions: {
        ecmaFeatures: { jsx: true },
      },
    },
  },
];

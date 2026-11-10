import globals from "globals";
import library from "./library.js";

export default [
  ...library,
  {
    files: ["**/*.{ts,tsx}"],
    languageOptions: { globals: globals.browser },
  },
];

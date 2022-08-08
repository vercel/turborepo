import { RULES } from "../constants";

const config = {
  plugins: ["turbo"],
  rules: {
    [`turbo/${RULES.noUndeclaredEnvVars}`]: "error",
  },
};

export default config;

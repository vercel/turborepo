import { RULES } from "../constants";

const config = {
  plugins: ["turbo"],
  rules: {
    [RULES.noUndeclaredEnvVars]: "error",
  },
};

export default config;

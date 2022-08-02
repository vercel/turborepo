import { RULES } from "../constants";

const config = {
  plugins: ["turbo"],
  rules: {
    [RULES.noUncachedEnvVars]: "error",
  },
};

export default config;

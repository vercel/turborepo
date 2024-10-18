import { RULES } from "./constants";
import noUndeclaredEnvVars from "./rules/no-undeclared-env-vars";
import recommended from "./configs/recommended";

const rules = {
  [RULES.noUndeclaredEnvVars]: noUndeclaredEnvVars,
};

const configs = {
  recommended,
};

export { rules, configs };

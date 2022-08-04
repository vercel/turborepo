import { RULES } from "./constants";

// rules
import noUndeclaredEnvVars from "./rules/no-undeclared-env-vars";

// configs
import recommended from "./configs/recommended";

const rules = {
  [RULES.noUndeclaredEnvVars]: noUndeclaredEnvVars,
};

const configs = {
  recommended,
};

export { rules, configs };

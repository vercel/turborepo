import { RULES } from "./constants";

// rules
import noUncachedEnvVars from "./rules/no-uncached-env-vars";

// configs
import recommended from "./configs/recommended";

const rules = {
  [RULES.noUncachedEnvVars]: noUncachedEnvVars,
};

const configs = {
  recommended,
};

export { rules, configs };

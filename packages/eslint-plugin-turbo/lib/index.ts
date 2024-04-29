import { name, version } from "../package.json";
import { RULES } from "./constants";
// rules
import noUndeclaredEnvVars from "./rules/no-undeclared-env-vars";
// configs
import recommended from "./configs/recommended";
import flatRecommended from "./configs/flat/recommended";

// See https://eslint.org/docs/latest/extend/plugins#meta-data-in-plugins
const meta = {
  name,
  version,
};

const rules = {
  [RULES.noUndeclaredEnvVars]: noUndeclaredEnvVars,
};

const configs = {
  recommended,
  "flat/recommended": flatRecommended,
};

export { meta, rules, configs };

/* Each pattern reads a distinct env var so lint warnings map 1:1 to patterns. */
const a = process.env.V01_DIRECT;                       // 1 direct member
const b = process.env["V02_BRACKET_LITERAL"];           // 2 bracket, string literal
const { V03_DESTRUCTURE } = process.env;                // 3 destructuring
const { V04_RENAMED: renamed } = process.env;           // 4 destructuring w/ rename
const env = process.env;                                // 5 aliasing
const c = env.V05_ALIAS;
const key = "V06_" + "CONCAT";                          // 6 constant-foldable concat
const d = process.env[key];
const e = process.env[`V07_${"TPL"}`];                  // 7 template literal
const getEnv = (k) => process.env[k];                   // 8 helper indirection
const f = getEnv("V08_WRAPPED");
const g = Reflect.get(process.env, "V09_REFLECT");      // 9 Reflect.get
const h = "V10_IN_CHECK" in process.env;                // 10 `in` existence check
const i = Object.keys(process.env).filter((k) =>        // 11 prefix scan
  k.startsWith("V11_PREFIX_")
);
const j = require("./indirect");                        // 12 read in another module

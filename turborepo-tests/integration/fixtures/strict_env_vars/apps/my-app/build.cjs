const { writeFileSync } = require("node:fs");

const env = process.env;
const fields = [
  ["globalpt", env.GLOBAL_VAR_PT || ""],
  ["localpt", env.LOCAL_VAR_PT || ""],
  ["globaldep", env.GLOBAL_VAR_DEP || ""],
  ["localdep", env.LOCAL_VAR_DEP || ""],
  ["other", env.OTHER_VAR || ""],
  ["sysroot set", env.SYSTEMROOT ? "yes" : "no"],
  ["path set", env.PATH ? "yes" : "no"],
];

writeFileSync(
  "out.txt",
  fields.map(([key, value]) => `${key}: '${value}'`).join(", ") + "\n",
);

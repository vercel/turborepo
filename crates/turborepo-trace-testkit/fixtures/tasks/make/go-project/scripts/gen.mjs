import { readFileSync } from "node:fs";
const spec = readFileSync(process.argv[2], "utf8");
console.log(`package api\n\nvar Routes = ${JSON.stringify(spec.match(/\/\w+/g))}`);

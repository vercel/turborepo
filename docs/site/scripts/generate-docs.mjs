import { generateFiles } from "fumadocs-openapi";

const out = "./content/openapi";

void generateFiles({
  input: ["https://turbo.build/api/remote-cache-spec"],
  output: out,
  groupBy: "tag",
});

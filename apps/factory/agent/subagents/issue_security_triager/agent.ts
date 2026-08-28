import { defineAgent } from "eve";

export default defineAgent({
  description:
    "Perform the mandatory tool-less security review of a newly opened Turborepo issue before the root agent inspects or executes its reproduction.",
  model: "anthropic/claude-fable-5"
});

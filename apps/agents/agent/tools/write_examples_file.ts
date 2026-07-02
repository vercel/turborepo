import { defineTool } from "eve/tools";
import { z } from "zod";

import { writeExamplesFile } from "../lib/repo.js";

export default defineTool({
  description:
    "Create or overwrite a repository file under examples/. Use this directly for example maintenance writes.",
  inputSchema: z.object({
    path: z
      .string()
      .min(1)
      .describe("Repository-relative file path under examples/."),
    content: z.string().describe("Complete file contents to write.")
  }),
  async execute({ path, content }) {
    return writeExamplesFile(path, content);
  }
});

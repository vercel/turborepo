import { defineAgent, defineDynamic } from "eve";

import { selectPerformanceModels } from "./lib/performance-models.js";
import { sessionDate } from "./lib/repo.js";

export default defineAgent({
  model: defineDynamic({
    fallback: "openai/gpt-5.6-sol",
    events: {
      "session.started": (_event, ctx) =>
        selectPerformanceModels(sessionDate(ctx.session.id)).authorModel
    }
  })
});

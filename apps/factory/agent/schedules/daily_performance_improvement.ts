import { defineSchedule } from "eve/schedules";

import { DAILY_PERFORMANCE_IMPROVEMENT_PROMPT } from "../lib/daily-performance-improvement.js";

export default defineSchedule({
  cron: "30 15 * * *",
  markdown: DAILY_PERFORMANCE_IMPROVEMENT_PROMPT
});

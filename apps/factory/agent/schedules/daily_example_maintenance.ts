import { defineSchedule } from "eve/schedules";

import { DAILY_EXAMPLE_MAINTENANCE_PROMPT } from "../lib/daily-example-maintenance.js";

export default defineSchedule({
  cron: "0 14 * * *",
  markdown: DAILY_EXAMPLE_MAINTENANCE_PROMPT
});

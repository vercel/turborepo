import { defineSchedule } from "eve/schedules";

import { WEEKLY_EXAMPLES_MAINTENANCE_PROMPT } from "../lib/weekly-examples-maintenance.js";

export default defineSchedule({
  cron: "0 14 * * 1",
  markdown: WEEKLY_EXAMPLES_MAINTENANCE_PROMPT
});

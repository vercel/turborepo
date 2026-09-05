import { defineSchedule } from "eve/schedules";

import { reconcileFactoryImageBuilds } from "../lib/factory-image-trigger.js";

export default defineSchedule({
  cron: "* * * * *",
  run({ waitUntil }) {
    waitUntil(reconcileFactoryImageBuilds());
  }
});

"use server";

import { runAuditAndFix } from "@/lib/audit";

export async function triggerAudit() {
  // Fire and forget — results go to Slack
  runAuditAndFix().catch((error) => {
    console.error("Audit failed:", error);
  });
}

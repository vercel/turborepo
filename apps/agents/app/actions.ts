"use server";

import { waitUntil } from "@vercel/functions";
import { runAuditAndFix } from "@/lib/audit";

export async function triggerAudit() {
  waitUntil(runAuditAndFix());
}
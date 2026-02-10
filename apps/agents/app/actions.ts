"use server";

import { cronSecret } from "@/lib/env";

export async function triggerAudit() {
  const baseUrl = process.env.VERCEL_URL
    ? `https://${process.env.VERCEL_URL}`
    : "http://localhost:3000";

  const res = await fetch(`${baseUrl}/api/cron/audit`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ secret: cronSecret() })
  });

  if (!res.ok) {
    throw new Error(`Failed to trigger audit: ${res.status}`);
  }

  return res.json();
}

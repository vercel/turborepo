import { execSync } from "node:child_process";
import dns from "node:dns";

const DNS_TIMEOUT = 5000;
const DNS_HOST = "github.com";

type DnsResult = "resolved" | "timeout" | "error";

export type OnlineStatus =
  | { online: true }
  | { online: false; reasons: string[] };

function getProxy(): string | undefined {
  if (process.env.https_proxy || process.env.HTTPS_PROXY) {
    return process.env.https_proxy || process.env.HTTPS_PROXY;
  }

  try {
    const httpsProxy = execSync("npm config get https-proxy", {
      timeout: 3000
    })
      .toString()
      .trim();
    return httpsProxy !== "null" ? httpsProxy : undefined;
  } catch (_) {
    // do nothing
  }
}

function dnsLookupWithTimeout(
  hostname: string,
  timeout: number
): Promise<DnsResult> {
  return new Promise((resolve) => {
    let settled = false;

    const timeoutId = setTimeout(() => {
      if (!settled) {
        settled = true;
        resolve("timeout");
      }
    }, timeout);

    dns.lookup(hostname, (err) => {
      if (!settled) {
        settled = true;
        clearTimeout(timeoutId);
        resolve(err === null ? "resolved" : "error");
      }
    });
  });
}

function describeDnsFailure(hostname: string, result: DnsResult): string {
  if (result === "timeout") {
    return `DNS lookup for "${hostname}" timed out after ${DNS_TIMEOUT / 1000}s`;
  }
  return `DNS lookup for "${hostname}" failed`;
}

export async function isOnline(): Promise<OnlineStatus> {
  const dnsResult = await dnsLookupWithTimeout(DNS_HOST, DNS_TIMEOUT);
  if (dnsResult === "resolved") {
    return { online: true };
  }

  const reasons: string[] = [describeDnsFailure(DNS_HOST, dnsResult)];

  const proxy = getProxy();
  if (!proxy) {
    reasons.push("No HTTPS proxy was detected as a fallback.");
    return { online: false, reasons };
  }

  let hostname: string | undefined;
  try {
    ({ hostname } = new URL(proxy));
  } catch {
    reasons.push(`HTTPS proxy "${proxy}" was detected but has an invalid URL.`);
    return { online: false, reasons };
  }
  if (!hostname) {
    reasons.push(`HTTPS proxy "${proxy}" was detected but has no hostname.`);
    return { online: false, reasons };
  }

  const proxyResult = await dnsLookupWithTimeout(hostname, DNS_TIMEOUT);
  if (proxyResult === "resolved") {
    return { online: true };
  }

  reasons.push(
    `HTTPS proxy "${proxy}" was detected but ${describeDnsFailure(hostname, proxyResult).toLowerCase()}.`
  );
  return { online: false, reasons };
}

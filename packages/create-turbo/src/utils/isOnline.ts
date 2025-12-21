import { execSync } from "node:child_process";
import dns from "node:dns";
import url from "node:url";

const DNS_TIMEOUT = 5000;

function getProxy(): string | undefined {
  if (process.env.https_proxy) {
    return process.env.https_proxy;
  }

  try {
    const httpsProxy = execSync("npm config get https-proxy", {
      timeout: 3000,
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
): Promise<boolean> {
  return new Promise((resolve) => {
    const timeoutId = setTimeout(() => {
      resolve(false);
    }, timeout);

    dns.lookup(hostname, (err) => {
      clearTimeout(timeoutId);
      resolve(err === null);
    });
  });
}

export async function isOnline(): Promise<boolean> {
  const registryOnline = await dnsLookupWithTimeout(
    "registry.yarnpkg.com",
    DNS_TIMEOUT
  );
  if (registryOnline) {
    return true;
  }

  const proxy = getProxy();
  if (!proxy) {
    return false;
  }

  const { hostname } = url.parse(proxy);
  if (!hostname) {
    return false;
  }

  return dnsLookupWithTimeout(hostname, DNS_TIMEOUT);
}

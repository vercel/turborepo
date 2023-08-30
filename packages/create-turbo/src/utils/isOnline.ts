import { execSync } from "node:child_process";
import dns from "node:dns";
import url from "node:url";

function getProxy(): string | undefined {
  if (process.env.https_proxy) {
    return process.env.https_proxy;
  }

  try {
    const httpsProxy = execSync("npm config get https-proxy").toString().trim();
    return httpsProxy !== "null" ? httpsProxy : undefined;
  } catch (_) {
    // do nothing
  }
}

export function isOnline(): Promise<boolean> {
  return new Promise((resolve) => {
    dns.lookup("registry.yarnpkg.com", (registryErr) => {
      if (!registryErr) {
        resolve(true);
        return;
      }

      const proxy = getProxy();
      if (!proxy) {
        resolve(false);
        return;
      }

      const { hostname } = url.parse(proxy);
      if (!hostname) {
        resolve(false);
        return;
      }

      dns.lookup(hostname, (proxyErr) => {
        resolve(proxyErr === null);
      });
    });
  });
}

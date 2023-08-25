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
        return resolve(true);
      }

      const proxy = getProxy();
      if (!proxy) {
        return resolve(false);
      }

      const { hostname } = url.parse(proxy);
      if (!hostname) {
        return resolve(false);
      }

      dns.lookup(hostname, (proxyErr) => {
        resolve(proxyErr === null);
      });
    });
  });
}

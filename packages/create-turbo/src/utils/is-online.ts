import https from "node:https";

const TIMEOUT = 5000;
const HOST = "https://github.com";

type OnlineStatus = { online: true } | { online: false; reason: string };

// Uses https.get which goes through the ProxyAgent
// already configured as https.globalAgent in cli.ts.
export function isOnline(): Promise<OnlineStatus> {
  return new Promise((resolve) => {
    const req = https.get(HOST, { timeout: TIMEOUT }, (res) => {
      res.resume();
      resolve({ online: true });
    });

    req.on("error", (err) => {
      resolve({ online: false, reason: err.message });
    });

    req.on("timeout", () => {
      req.destroy();
      resolve({
        online: false,
        reason: `Request to ${HOST} timed out after ${TIMEOUT / 1000}s`
      });
    });
  });
}

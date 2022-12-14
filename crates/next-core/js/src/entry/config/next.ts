import IPC, { Ipc } from "@vercel/turbopack-next/internal/ipc";
import loadConfig from "next/dist/server/config";
import { PHASE_DEVELOPMENT_SERVER } from "next/dist/shared/lib/constants";

type IpcIncomingMessage = {
  type: "loadConfig";
  path: string;
  configType: "next.config";
};

type IpcOutgoingMessage = {
  type: "javaScriptValue";
  data: number[];
};

const ipc = IPC as Ipc<IpcIncomingMessage, IpcOutgoingMessage>;

(async () => {
  while (true) {
    const msg = await ipc.recv();

    switch (msg.type) {
      case "loadConfig": {
        switch (msg.configType) {
          case "next.config":
            const nextConfig = await loadConfig(
              PHASE_DEVELOPMENT_SERVER,
              msg.path
            );
            // @ts-expect-error
            nextConfig.rewrites = await nextConfig.rewrites?.();
            // @ts-expect-error
            nextConfig.redirects = await nextConfig.redirects?.();
            await ipc.send({
              type: "javaScriptValue",
              data: Array.from(Buffer.from(JSON.stringify(nextConfig))),
            });
            break;
          default: {
            console.error("unexpected config type", msg.configType ?? msg);
            process.exit(1);
          }
        }
        break;
      }
      default: {
        console.error("unexpected message type", msg.type);
        process.exit(1);
      }
    }
  }
})().catch((err) => {
  ipc.sendError(err);
});

import { IPC } from "@vercel/turbopack-node-ipc";
const ipc = IPC;
(async () => {
  while (true) {
    const msg = await ipc.recv();
    switch (msg.type) {
      case "evaluate": {
        const { execute } = await import(msg.filepath);
        if (typeof execute !== "function") {
          console.error(
            `Expected ${msg.filepath} to export a function named "execute"`
          );
          process.exit(1);
        }
        const value = await execute.apply(null, msg.arguments);
        await ipc.send({
          type: "javaScriptValue",
          data: Array.from(Buffer.from(JSON.stringify(value))),
        });
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

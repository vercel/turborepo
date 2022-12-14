import net from "node:net";
import { parse as parseStackTrace } from "stacktrace-parser";
export function structuredError(e) {
  return {
    name: e.name,
    message: e.message,
    stack: parseStackTrace(e.stack),
  };
}
function createIpc(port) {
  const socket = net.createConnection(port, "127.0.0.1");
  const packetQueue = [];
  const recvPromiseResolveQueue = [];
  function pushPacket(packet) {
    const recvPromiseResolve = recvPromiseResolveQueue.shift();
    if (recvPromiseResolve != null) {
      recvPromiseResolve(JSON.parse(packet.toString("utf8")));
    } else {
      packetQueue.push(packet);
    }
  }
  let state = { type: "waiting" };
  let buffer = Buffer.alloc(0);
  socket.once("connect", () => {
    socket.on("data", (chunk) => {
      buffer = Buffer.concat([buffer, chunk]);
      loop: while (true) {
        switch (state.type) {
          case "waiting": {
            if (buffer.length >= 4) {
              const length = buffer.readUInt32BE(0);
              buffer = buffer.subarray(4);
              state = { type: "packet", length };
            } else {
              break loop;
            }
            break;
          }
          case "packet": {
            if (buffer.length >= state.length) {
              const packet = buffer.subarray(0, state.length);
              buffer = buffer.subarray(state.length);
              state = { type: "waiting" };
              pushPacket(packet);
            } else {
              break loop;
            }
            break;
          }
        }
      }
    });
  });
  function send(message) {
    const packet = Buffer.from(JSON.stringify(message), "utf8");
    const length = Buffer.alloc(4);
    length.writeUInt32BE(packet.length);
    socket.write(length);
    return new Promise((resolve, reject) => {
      socket.write(packet, (err) => {
        if (err != null) {
          reject(err);
        } else {
          resolve();
        }
      });
    });
  }
  return {
    async recv() {
      const packet = packetQueue.shift();
      if (packet != null) {
        return JSON.parse(packet.toString("utf8"));
      }
      const result = await new Promise((resolve) => {
        recvPromiseResolveQueue.push((result) => {
          resolve(result);
        });
      });
      return result;
    },
    send(message) {
      return send(message);
    },
    async sendError(error) {
      try {
        await send({
          type: "error",
          ...structuredError(error),
        });
      } catch (err) {
        // ignore and exit anyway
      }
      process.exit(1);
    },
  };
}
const PORT = process.argv[2];
export const IPC = createIpc(parseInt(PORT, 10));
process.on("uncaughtException", (err) => {
  IPC.sendError(err);
});

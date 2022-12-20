import { IPC, Ipc } from "./index";

type IpcIncomingMessage = {
  type: "evaluate";
  args: string[];
};

type IpcOutgoingMessage =
  | {
      type: "jsonValue";
      data: string;
    }
  | {
      type: "file-dependency";
      path: string;
    }
  | {
      type: "build-dependency";
      path: string;
    }
  | {
      type: "dir-dependency";
      path: string;
    };

const ipc = IPC as Ipc<IpcIncomingMessage, IpcOutgoingMessage>;

export const run = async (
  getValue: (
    ipc: Ipc<IpcIncomingMessage, IpcOutgoingMessage>,
    ...deserializedArgs: any[]
  ) => any
) => {
  while (true) {
    const msg = await ipc.recv();

    switch (msg.type) {
      case "evaluate": {
        try {
          const value = await getValue(ipc, ...msg.args);
          await ipc.send({
            type: "jsonValue",
            data: JSON.stringify(value),
          });
        } catch (e) {
          await ipc.sendError(e as Error);
        }
        break;
      }
      default: {
        console.error("unexpected message type", msg.type);
        process.exit(1);
      }
    }
  }
};

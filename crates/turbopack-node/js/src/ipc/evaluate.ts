import { IPC, Ipc } from "./index";

type IpcIncomingMessage = {
  type: "evaluate";
  filepath: string;
  arguments: string[];
};

type IpcOutgoingMessage = {
  type: "jsonValue";
  data: string;
};

const ipc = IPC as Ipc<IpcIncomingMessage, IpcOutgoingMessage>;

export const run = async (
  getValue: (...deserializedArgs: any[]) => any,
  ...args: string[]
) => {
  while (true) {
    const msg = await ipc.recv();

    switch (msg.type) {
      case "evaluate": {
        const deserializedArgs = args.map((arg) => JSON.parse(arg));
        const value = await getValue(...deserializedArgs).catch(
          (err: Error) => {
            return ipc.sendError(err);
          }
        );
        await ipc.send({
          type: "jsonValue",
          data: JSON.stringify(value),
        });
        break;
      }
      default: {
        console.error("unexpected message type", msg.type);
        process.exit(1);
      }
    }
  }
};

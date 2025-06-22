import { createServer } from "./server";
import { startFastifyServer } from "@repo/fastify";
import { log } from "@repo/logger";

const port = Number(process.env.PORT) || 3002;
const host = process.env.HOST || "0.0.0.0";

const server = createServer();

startFastifyServer(server, { port, host }).catch((err) => {
  log(`Failed to start server: ${err}`);
  process.exit(1);
});

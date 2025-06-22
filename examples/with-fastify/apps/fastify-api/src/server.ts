import { createFastifyApp, FastifyInstance } from "@repo/fastify";

export const createServer = (): FastifyInstance => {
  const server = createFastifyApp({
    logger: true,
    cors: true,
    helmet: true,
  });

  // Health check endpoint
  server.get("/health", async (request, reply) => {
    return { status: "ok", timestamp: new Date().toISOString() };
  });

  // Message endpoint
  server.get("/message/:name", async (request, reply) => {
    const { name } = request.params as { name: string };
    return { message: `Hello ${name} from Fastify!` };
  });

  // Status endpoint
  server.get("/status", async (request, reply) => {
    return {
      ok: true,
      service: "fastify-api",
      uptime: process.uptime(),
    };
  });

  return server;
};

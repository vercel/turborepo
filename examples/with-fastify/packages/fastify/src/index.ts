import Fastify, { FastifyInstance, FastifyServerOptions } from "fastify";
import cors from "@fastify/cors";
import helmet from "@fastify/helmet";

export interface FastifyAppOptions {
  logger?: boolean;
  cors?: boolean;
  helmet?: boolean;
  port?: number;
  host?: string;
}

export function createFastifyApp(
  options: FastifyAppOptions = {},
): FastifyInstance {
  const serverOptions: FastifyServerOptions = {
    logger: options.logger ?? true,
  };

  const server = Fastify(serverOptions);

  // Register common plugins
  if (options.cors !== false) {
    server.register(cors, {
      origin: true,
      credentials: true,
    });
  }

  if (options.helmet !== false) {
    server.register(helmet);
  }

  return server;
}

export async function startFastifyServer(
  server: FastifyInstance,
  options: { port?: number; host?: string } = {},
): Promise<void> {
  try {
    const address = await server.listen({
      port: options.port ?? 3000,
      host: options.host ?? "0.0.0.0",
    });
    server.log.info(`Server listening at ${address}`);
  } catch (err) {
    server.log.error(err);
    process.exit(1);
  }
}

// Export Fastify types for convenience
export type {
  FastifyInstance,
  FastifyRequest,
  FastifyReply,
  RouteHandlerMethod,
} from "fastify";

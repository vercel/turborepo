import { describe, it, expect } from "@jest/globals";
import { createServer } from "../server";

describe("fastify server", () => {
  it("health check returns 200", async () => {
    const server = createServer();

    const response = await server.inject({
      method: "GET",
      url: "/health",
    });

    expect(response.statusCode).toBe(200);
    const payload = JSON.parse(response.payload);
    expect(payload.status).toBe("ok");
  });

  it("message endpoint says hello", async () => {
    const server = createServer();

    const response = await server.inject({
      method: "GET",
      url: "/message/world",
    });

    expect(response.statusCode).toBe(200);
    const payload = JSON.parse(response.payload);
    expect(payload.message).toBe("Hello world from Fastify!");
  });

  it("status endpoint returns ok", async () => {
    const server = createServer();

    const response = await server.inject({
      method: "GET",
      url: "/status",
    });

    expect(response.statusCode).toBe(200);
    const payload = JSON.parse(response.payload);
    expect(payload.ok).toBe(true);
    expect(payload.service).toBe("fastify-api");
  });
});

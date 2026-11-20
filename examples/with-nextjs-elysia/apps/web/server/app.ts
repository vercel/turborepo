import { Elysia } from "elysia";

/**
 * The Elysia app that serves every request under `/api/v1`.
 * It is mounted by the Next.js catch-all route handler in
 * `src/app/api/v1/[[...slugs]]/route.ts`.
 */
export const app = new Elysia({ name: "app", prefix: "/api/v1" }).get(
  "/",
  () => "Hello from Elysia"
);

export type App = typeof app;

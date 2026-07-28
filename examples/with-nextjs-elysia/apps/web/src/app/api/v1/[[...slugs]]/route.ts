import { Elysia } from 'elysia';
import { appRouter } from "~/server";
/**
 * 使用 server.ts 中定义的服务器实例
 * 设置 /api 前缀以匹配路由路径
 * 注意：启动检查已在 instrumentation.ts 中执行
 */
export const app = new Elysia({ name: "app", prefix: "/api" })
  .get('/', "dddddddd")
  .use(appRouter)


export const POST = app.handle;
export const PUT = app.handle;
export const DELETE = app.handle;
export const PATCH = app.handle;



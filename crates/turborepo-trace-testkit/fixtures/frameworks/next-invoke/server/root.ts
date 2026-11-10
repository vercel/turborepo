import { createCallerFactory, createTRPCRouter, publicProcedure } from './trpc';
import { userRouter } from './routers/user';
import { postRouter } from './routers/post';

export const appRouter = createTRPCRouter({
  user: userRouter,
  post: postRouter,
  greeting: publicProcedure.query(() => 'hello'),
});

export type AppRouter = typeof appRouter;
export const createCaller = createCallerFactory(appRouter);

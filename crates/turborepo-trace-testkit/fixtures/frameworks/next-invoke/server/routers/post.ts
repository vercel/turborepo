import { createTRPCRouter, publicProcedure } from '../trpc';

export const postRouter = createTRPCRouter({
  all: publicProcedure.query(() => []),
  comments: createTRPCRouter({
    add: publicProcedure.mutation(() => ({ ok: true })),
  }),
});

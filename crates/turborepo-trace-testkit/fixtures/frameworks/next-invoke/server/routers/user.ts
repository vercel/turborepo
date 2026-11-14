import { createTRPCRouter, publicProcedure } from '../trpc';

export const userRouter = createTRPCRouter({
  byId: publicProcedure
    .input((v: unknown) => v as { id: string })
    .query(({ input }) => ({ id: input.id, name: 'Ada' })),
  list: publicProcedure.query(() => []),
  create: publicProcedure
    .input((v: unknown) => v as { name: string })
    .mutation(({ input }) => ({ id: 'u2', name: input.name })),
});

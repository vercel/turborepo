import { defineConfig } from 'prisma/config';

export default defineConfig({
  schema: process.env.PRISMA_SCHEMA,
});

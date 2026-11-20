import type { Config } from 'drizzle-kit';

export default {
  schema: ['./src/db/schema/users.ts', './src/db/relations.ts'],
  out: './migrations',
  dialect: 'sqlite',
} satisfies Config;

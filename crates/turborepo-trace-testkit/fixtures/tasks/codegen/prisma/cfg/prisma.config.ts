import path from 'node:path';
import { defineConfig } from 'prisma/config';

export default defineConfig({
  schema: path.join('db', 'schema.prisma'),
  migrations: {
    path: 'db/migrations',
    seed: 'node db/seed.js',
  },
});

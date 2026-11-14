import { defineConfig } from 'drizzle-kit';
import { schemas } from './schemas';

export default defineConfig({
  schema: schemas(),
  out: process.env.OUT_DIR,
  dialect: 'mysql',
});

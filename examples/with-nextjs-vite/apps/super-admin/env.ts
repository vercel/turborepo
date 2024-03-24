import { defineConfig } from '@julr/vite-plugin-validate-env';
import { z } from 'zod';

export default defineConfig({
  validator: 'zod',
  schema: {
    VITE_AWS_REGION: z.string(),
  },
});

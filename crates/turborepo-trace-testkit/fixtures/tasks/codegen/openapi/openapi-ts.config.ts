import { defineConfig } from '@hey-api/openapi-ts';

export default defineConfig({
  input: './openapi.yaml',
  output: 'src/client',
  plugins: ['@hey-api/client-fetch'],
});

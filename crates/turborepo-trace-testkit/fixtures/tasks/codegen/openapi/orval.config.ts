import { defineConfig } from 'orval';

export default defineConfig({
  petstore: {
    input: { target: './openapi.yaml' },
    output: {
      mode: 'tags-split',
      target: 'src/orval/petstore.ts',
      schemas: 'src/orval/model',
      client: 'react-query',
    },
  },
  simple: {
    input: './openapi.yaml',
    output: './src/simple.ts',
  },
});

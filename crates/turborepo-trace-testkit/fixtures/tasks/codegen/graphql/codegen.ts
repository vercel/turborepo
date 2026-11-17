import type { CodegenConfig } from '@graphql-codegen/cli';

const config: CodegenConfig = {
  schema: 'schema.graphql',
  documents: ['src/**/*.graphql', '!src/legacy/**'],
  generates: {
    'src/gql/': {
      preset: 'client',
    },
    './src/types.ts': {
      plugins: ['typescript', 'typescript-operations'],
    },
  },
};

export default config;

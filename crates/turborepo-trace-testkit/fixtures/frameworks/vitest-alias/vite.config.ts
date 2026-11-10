import { defineConfig } from 'vite';
import path from 'node:path';

export default defineConfig({
  resolve: {
    alias: [
      { find: /^~icons\/(.*)$/, replacement: path.resolve(__dirname, 'src/icons/$1.svg') },
      { find: '@lib', replacement: path.resolve(__dirname, 'src/lib') },
    ],
  },
  plugins: [],
});

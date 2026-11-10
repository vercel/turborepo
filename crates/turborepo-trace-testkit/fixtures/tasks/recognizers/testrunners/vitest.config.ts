import { defineConfig } from 'vitest/config';
export default defineConfig({ test: { include: ['src/**/*.spec.ts'], setupFiles: ['./test/setup.ts'] } });

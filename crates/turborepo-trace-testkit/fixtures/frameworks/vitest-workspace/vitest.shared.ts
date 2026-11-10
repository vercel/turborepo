export default {
  test: { setupFiles: ['./vitest.setup.ts'], globals: true },
  resolve: { alias: { '#root': '/abs/root' } },
};

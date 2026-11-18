// A React app whose tests import modules that statically import `.wgsl` shaders: next to
// the JSX plugin, a loader plugin compiles the shader modules (inlining the shaders they
// import), as the production bundler's loader does.
import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import wgslVitePlugin from '@vgpu/wgsl/loader-vite';

export default defineConfig({
  plugins: [react(), wgslVitePlugin({ minify: true })],
  test: {
    globals: true,
    environment: 'jsdom',
  },
});

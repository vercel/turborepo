// Tests that import modules which statically import `.wgsl` shaders: next to the JSX
// plugin, a loader plugin compiles the shader modules (inlining the shaders they import),
// as the app's bundler does.
import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import wgsl from '@vgpu/wgsl/loader-vite';

export default defineConfig({
  test: {
    environment: 'jsdom',
    globals: true,
  },
  plugins: [react({ jsxRuntime: 'automatic' }), wgsl()],
});

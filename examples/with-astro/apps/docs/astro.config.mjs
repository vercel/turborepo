import { defineConfig } from "astro/config";
import react from "@astrojs/react";

import svelte from "@astrojs/svelte";

// https://astro.build/config
export default defineConfig({
  server: {
    port: 3001
  },
  integrations: [react(), svelte()]
});
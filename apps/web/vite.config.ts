import { svelte } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [svelte()],
  build: {
    rolldownOptions: {
      output: {
        // The Svelte runtime changes only on dependency upgrades, so it gets a
        // long-lived content-hashed chunk separate from application code.
        // PlayerDashboard is split by its dynamic import (lib/app/lazy-dashboard.ts).
        codeSplitting: {
          groups: [
            { name: 'svelte-runtime', test: /[\\/]node_modules[\\/](?:\.pnpm[\\/][^\\/]+[\\/]node_modules[\\/])?svelte[\\/]/ },
          ],
        },
      },
    },
  },
  server: {
    proxy: {
      '/api': {
        target: 'http://127.0.0.1:8080',
      },
      '/ws': {
        target: 'ws://127.0.0.1:8080',
        ws: true,
      },
    },
  },
});

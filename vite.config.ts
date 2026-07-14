import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [sveltekit()],
  clearScreen: false,
  server: {
    strictPort: true,
    port: 1420
  },
  envPrefix: ['VITE_', 'TAURI_']
});


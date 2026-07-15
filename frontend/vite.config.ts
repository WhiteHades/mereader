import { svelte } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vite';

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  envPrefix: ['VITE_', 'TAURI_'],
  server: {
    host: host || false,
    port: 5173,
    strictPort: true,
    hmr: host
      ? {
          protocol: 'ws',
          host,
          port: 1421,
        }
      : undefined,
  },
  build: {
    target: ['es2021', 'chrome100', 'safari13'],
    minify: process.env.TAURI_DEBUG ? false : 'oxc',
    sourcemap: Boolean(process.env.TAURI_DEBUG),
  },
});

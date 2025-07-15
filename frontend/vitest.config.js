import { defineConfig } from 'vitest/config';
import vue from '@vitejs/plugin-vue';
import { resolve } from 'path';

export default defineConfig({
  plugins: [
    vue(),
  ],
  test: {
    environment: 'jsdom',
    globals: true,
    include: ['src/tests/**/*.spec.js'],
    coverage: {
      reporter: ['text', 'html'],
      exclude: [
        'node_modules/',
        'src/tests/',
      ],
    },
    deps: {
      inline: ['@tauri-apps/api']
    },
    setupFiles: ['src/tests/setup.js']
  },
  resolve: {
    alias: {
      '@': resolve(__dirname, './src')
    }
  }
});
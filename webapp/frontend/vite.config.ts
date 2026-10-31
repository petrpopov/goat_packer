import react from '@vitejs/plugin-react';
import { fileURLToPath, URL } from 'node:url';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  build: {
    // Порог по умолчанию (500 КБ) срабатывает на чанк three.js; он ленивый (сцена грузится только
    // на странице упаковки), главный бандл от него не растёт.
    chunkSizeWarningLimit: 700,
  },
  server: {
    port: 3007,
    proxy: {
      '/api': {
        target: process.env.GOAT_API_TARGET ?? 'http://localhost:19190',
        changeOrigin: true,
      },
    },
  },
});

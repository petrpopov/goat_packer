import react from '@vitejs/plugin-react';
import { fileURLToPath, URL } from 'node:url';
import { execSync } from 'node:child_process';
import { defineConfig } from 'vite';

/** Версия и коммит сборки (в углу меню): образ получает их build-arg'ами, dev — из git. */
function gitCommit(): string {
  try {
    return execSync('git rev-parse --short HEAD', { stdio: ['ignore', 'pipe', 'ignore'] }).toString().trim();
  } catch {
    return '';
  }
}
const build = { version: process.env.GOAT_VERSION || 'dev', commit: process.env.GOAT_COMMIT || gitCommit() };

export default defineConfig({
  plugins: [react()],
  define: { __GOAT_BUILD__: JSON.stringify(build) },
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  build: {
    // Шрифты — всегда файлами: мелкие подмножества Vite встроил бы как data:, а CSP gateway
    // (font-src 'self') их блокирует — часть глифов на проде падала на системный шрифт.
    assetsInlineLimit: (file) => (/\.(woff2?|ttf|otf)$/.test(file) ? false : undefined),
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

import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vite';
import solid from 'vite-plugin-solid';

export default defineConfig(() => {
  for (const file of ['oneq.js', 'oneq_bg.wasm']) {
    if (!existsSync(new URL(`../web/pkg/${file}`, import.meta.url))) {
      throw new Error(
        'Build the WASM package first: run the build:wasm script in web-playground.',
      );
    }
  }
  return {
    base: './',
    plugins: [solid()],
    worker: { format: 'es' as const },
    server: {
      fs: { allow: [fileURLToPath(new URL('..', import.meta.url))] },
    },
  };
});

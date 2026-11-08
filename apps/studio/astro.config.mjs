import { defineConfig } from 'astro/config';
import react from '@astrojs/react';
import tailwindcss from '@tailwindcss/vite';

const backend = process.env.RDF_BACKEND_URL || 'http://127.0.0.1:7878';
export default defineConfig({
  output: 'static',
  devToolbar: { enabled: false },
  integrations: [react()],
  vite: {
    plugins: [tailwindcss()],
    server: { proxy: Object.fromEntries(['/api', '^/d/', '/query', '/update', '/store', '/health', '/metrics'].map(path => [path, { target: backend }])) },
  },
});

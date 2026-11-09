import { defineConfig } from 'astro/config';
import react from '@astrojs/react';
import starlight from '@astrojs/starlight';
import starlightLinksValidator from 'starlight-links-validator';
import tailwindcss from '@tailwindcss/vite';

const backend = process.env.RDF_BACKEND_URL || 'http://127.0.0.1:7878';
export default defineConfig({
  output: 'static',
  devToolbar: { enabled: false },
  integrations: [
    react(),
    // The documentation lives in the same app, under /docs (see docs/specs/studio-docs.md).
    starlight({
      title: 'RDF Studio docs',
      description: 'An RDF store built on Oxigraph, with a plugin architecture. What it is, how to use it, and how it works.',
      customCss: ['./src/styles/docs.css'],
      social: [{ icon: 'github', label: 'Source on GitHub', href: 'https://github.com/VincentK1991/being-in-the-world' }],
      sidebar: [
        { label: 'Back to Studio', link: '/' },
        { label: 'What it is', items: [{ autogenerate: { directory: 'docs/what-it-is' } }] },
        { label: 'How to use', items: [{ autogenerate: { directory: 'docs/how-to-use' } }] },
        { label: 'Internals', items: [{ autogenerate: { directory: 'docs/internals' } }] },
      ],
      plugins: [
        // Fails the build on a broken internal link. Studio's own pages are not in the content
        // collection, so they are listed here.
        starlightLinksValidator({ exclude: ['/', '/datasets', '/datasets/', '/workspace', '/workspace/', '/plugins', '/plugins/'] }),
      ],
    }),
  ],
  vite: {
    plugins: [tailwindcss()],
    server: { proxy: Object.fromEntries(['/api', '^/d/', '/query', '/update', '/store', '/health', '/metrics'].map(path => [path, { target: backend }])) },
  },
});

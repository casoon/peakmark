// @ts-check
import casoonPages from '@casoon/pages-theme';
import { defineConfig } from 'astro/config';

// Project page: https://casoon.github.io/<repo>/ — `base` is the GitHub Pages path.
export default defineConfig({
  site: 'https://casoon.github.io/peakmark',
  base: '/peakmark/',
  integrations: [
    casoonPages({
      name: 'peakmark',
      description:
        'Gate WebAssembly modules on size and memory against a baseline kept in the repository.',
      tagline: 'WebAssembly size and memory budgets',
      repo: 'casoon/peakmark',
      version: '0.1.1',
      license: 'MIT',
      catalog: false,
      packages: [{ label: 'crates.io', href: 'https://crates.io/crates/peakmark' }],
      docsGroups: {
        'getting-started': 'Getting started',
        concepts: 'Concepts',
        guides: 'Guides',
        reference: 'Reference',
      },
    }),
  ],
});

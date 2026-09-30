import { defineConfig } from 'vitepress'

// The docs site — sources live in docs/ (this file's parent), built to
// docs/.vitepress/dist and deployed to GitHub Pages at /tono/.
export default defineConfig({
  base: '/tono/',
  title: 'tono',
  description: 'Audio as a pure function — procedural, deterministic, CI-testable.',
  cleanUrls: true,
  lastUpdated: false,
  head: [['link', { rel: 'icon', href: '/tono/img/logo.png' }]],

  themeConfig: {
    logo: '/img/logo.png',
    nav: [
      { text: 'Get started', link: '/get-started/' },
      { text: 'Guides', link: '/guides/sound-effects' },
      { text: 'Reference', link: '/reference/sounddoc' },
      { text: 'Showcase', link: '/showcase' },
      {
        text: 'Project',
        items: [
          { text: 'Migration', link: '/project/migration' },
          { text: 'API stability tiers', link: '/project/api-tiers' },
          { text: 'Performance budgets', link: '/project/performance' },
          { text: 'Release gates', link: '/project/release-gates' },
          { text: 'Changelog', link: 'https://github.com/marmikshah/tono/blob/master/CHANGELOG.md' },
        ],
      },
    ],
    sidebar: {
      '/get-started/': [
        {
          text: 'Get started',
          items: [
            { text: 'Install', link: '/get-started/' },
            { text: 'Ten-minute quickstart', link: '/get-started/quickstart' },
          ],
        },
      ],
      '/guides/': [
        {
          text: 'Guides',
          items: [
            { text: 'Generate game SFX', link: '/guides/generation' },
            { text: 'Design sound effects', link: '/guides/sound-effects' },
            { text: 'Compose songs', link: '/guides/songs' },
            { text: 'Run live & embedded', link: '/guides/live' },
            { text: 'Python', link: '/guides/python' },
          ],
        },
      ],
      '/reference/': [
        {
          text: 'Reference',
          items: [
            { text: 'The SoundDoc nodes', link: '/reference/sounddoc' },
            { text: 'Determinism & streaming', link: '/reference/determinism' },
            { text: 'The CLI', link: '/reference/cli' },
            { text: 'Rust API (docs.rs)', link: 'https://docs.rs/tono-core' },
          ],
        },
      ],
      '/project/': [
        {
          text: 'Project',
          items: [
            { text: 'Migration', link: '/project/migration' },
            { text: 'API stability tiers', link: '/project/api-tiers' },
            { text: 'Performance budgets', link: '/project/performance' },
            { text: 'Release gates', link: '/project/release-gates' },
            { text: 'Architecture', link: 'https://marmikshah.github.io/tono/architecture.html' },
            { text: 'Design decisions (ADRs)', link: 'https://github.com/marmikshah/tono/tree/master/docs/adr' },
            { text: 'Changelog', link: 'https://github.com/marmikshah/tono/blob/master/CHANGELOG.md' },
          ],
        },
      ],
    },
    socialLinks: [{ icon: 'github', link: 'https://github.com/marmikshah/tono' }],
    search: { provider: 'local' },
    editLink: {
      pattern: 'https://github.com/marmikshah/tono/edit/master/docs/:path',
      text: 'Edit this page on GitHub',
    },
    outline: { level: [2, 3], label: 'On this page' },
    footer: { message: 'MIT licensed — permissive, no warranty.' },
  },
})

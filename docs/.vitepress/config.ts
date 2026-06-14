import { defineConfig } from 'vitepress'

const version = process.env.KUBARR_VERSION || '0.0.0'
const channel = process.env.KUBARR_CHANNEL || 'dev'
const commit = process.env.KUBARR_COMMIT || 'unknown'

export default defineConfig({
  title: 'Kubarr',
  description: 'Smooth sailing for self-hosted media on Kubernetes',
  base: '/Kubarr/',
  outDir: '../site',
  cleanUrls: true,
  lastUpdated: true,
  ignoreDeadLinks: [/^http:\/\/localhost(:\d+)?/],
  head: [
    ['meta', { name: 'theme-color', content: '#4f46e5' }]
  ],
  markdown: {
    config(md) {
      md.core.ruler.before('normalize', 'kubarr_version_placeholders', (state) => {
        state.src = state.src
          .replaceAll('{{VERSION}}', version)
          .replaceAll('{{CHANNEL}}', channel)
          .replaceAll('{{COMMIT}}', commit)
      })
    }
  },
  themeConfig: {
    logo: undefined,
    siteTitle: 'Kubarr',
    nav: [
      { text: 'Quick Start', link: '/quick-start' },
      { text: 'Architecture', link: '/architecture' },
      { text: 'GitHub', link: 'https://github.com/smokeythebandit/Kubarr' }
    ],
    sidebar: [
      {
        text: 'Getting Started',
        items: [
          { text: 'Home', link: '/' },
          { text: 'Quick Start', link: '/quick-start' },
          { text: 'Docker', link: '/docker' },
          { text: 'Configuration', link: '/configuration' },
          { text: 'Development', link: '/development' },
          { text: 'Versioning', link: '/versioning' }
        ]
      },
      {
        text: 'Architecture',
        items: [
          { text: 'Overview', link: '/architecture' },
          { text: 'Applications', link: '/applications' },
          { text: 'Networking', link: '/networking' },
          { text: 'Storage', link: '/storage' },
          { text: 'API', link: '/api' },
          { text: 'User Guide', link: '/user-guide' }
        ]
      },
      {
        text: 'ADRs',
        items: [
          { text: 'Index', link: '/adr/' },
          { text: 'Storage Model', link: '/adr/storage-model-architecture' }
        ]
      }
    ],
    socialLinks: [
      { icon: 'github', link: 'https://github.com/smokeythebandit/Kubarr' }
    ],
    search: {
      provider: 'local'
    },
    editLink: {
      pattern: 'https://github.com/smokeythebandit/Kubarr/edit/main/docs/:path',
      text: 'Edit this page on GitHub'
    },
    footer: {
      message: `Kubarr ${version} (${channel})`,
      copyright: `Commit ${commit}`
    }
  }
})

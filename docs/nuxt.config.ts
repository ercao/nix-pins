const baseURL = process.env.NUXT_APP_BASE_URL || '/'
const siteURL = process.env.NUXT_SITE_URL || 'https://nix-pins.ercao.dev'

export default defineNuxtConfig({
  extends: ['docus'],
  modules: ['@nuxtjs/i18n'],
  i18n: {
    defaultLocale: 'zh-CN',
    locales: [{ code: 'zh-CN', language: 'zh-CN', name: '简体中文' }],
    detectBrowserLanguage: false,
  },
  site: {
    name: 'nix-pins',
    url: siteURL,
  },
  app: {
    baseURL,
    head: {
      htmlAttrs: { lang: 'zh-CN' },
    },
  },
  llms: {
    domain: siteURL,
    full: false,
  },
  mcp: { enabled: false },
  docus: { assistant: { enabled: false } },
  ogImage: { enabled: false },
  devtools: { enabled: false },
  content: {
    experimental: { sqliteConnector: 'native' },
    build: {
      pathMeta: { slugifyOptions: { lower: false } },
      markdown: {
        highlight: { langs: ['nix', 'bash', 'json', 'toml', 'yaml'] },
      },
    },
  },
  nitro: {
    preset: 'static',
    prerender: {
      failOnError: true,
      routes: [
        '/zh-CN/getting-started/installation',
        '/zh-CN/getting-started',
        '/zh-CN/guides',
        '/zh-CN/reference',
        '/zh-CN/troubleshooting',
      ],
    },
  },
})

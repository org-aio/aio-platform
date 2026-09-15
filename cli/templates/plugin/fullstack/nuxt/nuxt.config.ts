export default defineNuxtConfig({
  compatibilityDate: '2026-09-15',
  srcDir: 'frontend',
  serverDir: 'backend/http',
  ssr: false,
  devtools: { enabled: false },
  app: { buildAssetsDir: '/_nuxt/', head: { title: "__TITLE__" } },
  nitro: { preset: 'node-server', prerender: { routes: ['/'] } },
});

export default {
  output: 'standalone',
  assetPrefix: '.',
  outputFileTracingRoot: new URL('..', import.meta.url).pathname,
  poweredByHeader: false,
  outputFileTracingExcludes: { '*': ['node_modules/sharp/**/*', 'node_modules/@img/**/*', 'node_modules/.pnpm/@img+*/**/*', 'node_modules/.pnpm/sharp@*/**/*'] },
  images: { unoptimized: true },
};

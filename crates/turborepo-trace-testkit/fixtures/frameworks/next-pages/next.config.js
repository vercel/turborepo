/** @type {import('next').NextConfig} */
module.exports = {
  basePath: '/shop',
  i18n: { locales: ['en', 'de'], defaultLocale: 'en' },
  async rewrites() {
    return [{ source: '/p/:id', destination: '/product/:id' }];
  },
  async redirects() {
    return process.env.LEGACY === '1'
      ? [{ source: '/old', destination: '/', permanent: false }]
      : [];
  },
};

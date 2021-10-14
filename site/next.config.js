module.exports = {
  images: {
    domains: ['images.ctfassets.net'],
  },
  experimental: {
    turboMode: true,
    eslint: true,
  },
  async rewrites() {
    return [
      {
        source: '/docs',
        destination: 'https://turborepo-docs.vercel.app/docs', // Matched parameters can be used in the destination
      },
      {
        source: '/docs/:path*',
        destination: 'https://turborepo-docs.vercel.app/docs/:path*', // Matched parameters can be used in the destination
      },
    ]
  },
}

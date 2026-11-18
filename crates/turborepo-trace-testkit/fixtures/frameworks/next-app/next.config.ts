import type { NextConfig } from 'next';
import withPlugin from 'some-next-plugin';
import { legacy } from './redirects.mjs';

const nextConfig: NextConfig = {
  pageExtensions: ['tsx', 'ts'],
  reactStrictMode: true,
  async redirects() {
    return [
      ...legacy,
      { source: '/old-blog/:slug', destination: '/blog/:slug', permanent: true },
      {
        source: '/beta',
        has: [{ type: 'cookie', key: 'beta' }],
        destination: '/dashboard',
        permanent: false,
      },
    ];
  },
  async rewrites() {
    return {
      beforeFiles: [{ source: '/docs/:path*', destination: 'https://docs.example.com/:path*' }],
      afterFiles: [
        { source: '/api/v1/:path*', destination: 'https://api.example.com/v1/:path*' },
        { source: '/team-alias/:team', destination: '/:team' },
      ],
      fallback: [],
    };
  },
};

// Plugin wrappers evaluate to unknown values; the loader reads `nextConfig`.
export default withPlugin(nextConfig);

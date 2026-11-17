/** @type {import('next').NextConfig} */
const config = {
  async redirects() {
    return [{ source: '/plans', destination: '/pricing', permanent: false }];
  },
};

export default config;

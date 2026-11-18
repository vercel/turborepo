import createMiddleware from 'intl-kit/middleware';

// Every page lives under `/[locale]`: the library's middleware rewrites `/login` to
// `/en/login`. Nothing in this file says where a request goes.
export default createMiddleware({ locales: ['en', 'de'], defaultLocale: 'en' });

// The documentation and the handlers are served as they are.
export const config = {
  matcher: [
    '/',
    '/login',
    '/login/:path*',
    '/new',
    '/poll/:path*',
    '/admin',
    '/admin/:path*',
    '/admin-setup',
    '/pricing',
  ],
};

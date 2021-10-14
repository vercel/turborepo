import { AppProps } from 'next/app'
import { SSRProvider } from '@react-aria/ssr'
import { ThemeProvider } from 'next-themes'
import '../styles/index.css'
import NextHead from 'next/head'
import { DefaultSeo } from 'next-seo'

const SEO = {
  titleTemplate: '%s | Turborepo',
  openGraph: {
    type: 'website',
    locale: 'en_IE',
    url: 'https://turborepo.com',
    site_name: 'Turborepo',
    images: [
      {
        url: 'https://turborepo.com/og-image.jpg',
      },
    ],
  },
  twitter: {
    handle: '@turborepo',
    site: '@turborepo',
    cardType: 'summary_large_image',
  },
}
export default function MyApp({ Component, pageProps }: AppProps) {
  return (
    <SSRProvider>
      <DefaultSeo {...SEO} />
      <NextHead>
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <link
          rel="apple-touch-icon"
          sizes="180x180"
          href="/favicon/apple-touch-icon.png"
        />
        <link
          rel="icon"
          type="image/png"
          sizes="32x32"
          href="/favicon/favicon-32x32.png"
        />
        <link
          rel="icon"
          type="image/png"
          sizes="16x16"
          href="/favicon/favicon-16x16.png"
        />
        <link
          rel="mask-icon"
          href="/favicon/safari-pinned-tab.svg"
          color="#000000"
        />
        <link rel="shortcut icon" href="/favicon/favicon.ico" />
        <meta name="msapplication-TileColor" content="#000000" />
        <meta
          name="msapplication-config"
          content="/favicon/browserconfig.xml"
        />
        <meta name="theme-color" content="#000" />
        {/* <link rel="alternate" type="application/rss+xml" href="/feed.xml" /> */}
      </NextHead>
      <ThemeProvider defaultTheme="dark" attribute="class">
        <Component {...pageProps} />
        <style jsx global>{`
          .cta {
            background: #000;
            color: #fff;
            position: relative;
            z-index: 1;
            border: none;
            transform-style: preserve-3d;
            transition: background 350ms ease-in-out, color 350ms ease-in-out;
            cursor: pointer;
            display: inline-block;
            text-align: center;
            white-space: nowrap;
            min-width: 28px;

            border-radius: 12px;
            color: #fff;
          }
          @media (hover: hover) {
            .cta:hover {
              color: #1d1d1f;
              background: #f5f5f7;
            }
          }
          .cta:after {
            content: '';
            display: block;
            position: absolute;
            top: -1px;
            left: -1px;
            width: calc(100% + 2.5px);
            height: calc(100% + 2px);
            background: linear-gradient(to right, #3b82f6 10%, #ef4444 90%);
            background-size: 100% 200%;
            transform: translateZ(-1px);
            border-radius: 12px;
            overflow: hidden;
          }
        `}</style>
      </ThemeProvider>
    </SSRProvider>
  )
}

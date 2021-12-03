import React from 'react'
import NextHead from 'next/head'

import renderComponent from './utils/render-component'
import { useConfig } from './config'

export default function Head({ title, locale, meta }) {
  const config = useConfig()
  return (
    <NextHead>
      {config.font ? (
        <link rel="stylesheet" href="https://rsms.me/inter/inter.css" />
      ) : null}
      <title>
        {title}
        {renderComponent(config.titleSuffix, { locale, config, title, meta })}
      </title>
      {config.font ? (
        <style
          dangerouslySetInnerHTML={{
            __html: `html{font-family:Inter,sans-serif}@supports(font-variation-settings:normal){html{font-family:'Inter var',sans-serif}}`
          }}
        />
      ) : null}
      {renderComponent(config.head, { locale, config, title, meta })}
      {config.unstable_faviconGlyph ? (
        <link
          rel="icon"
          href={`data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100'><text x='50' y='.9em' font-size='90' text-anchor='middle'>${config.unstable_faviconGlyph}</text><style>text{font-family:system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,"Helvetica Neue",Arial,"Noto Sans",sans-serif,"Apple Color Emoji","Segoe UI Emoji","Segoe UI Symbol","Noto Color Emoji";fill:black}@media(prefers-color-scheme:dark){text{fill:white}}</style></svg>`}
        />
      ) : null}
    </NextHead>
  )
}

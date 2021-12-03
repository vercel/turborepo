import React from 'react'
import cn from 'classnames'
import Link from 'next/link'
import { useRouter } from 'next/router'
import parseGitUrl from 'parse-git-url'

import ArrowRight from './icons/arrow-right'
import renderComponent from './utils/render-component'
import { useConfig } from './config'

const NextLink = ({ route, title, isRTL }) => {
  return (
    <Link href={route}>
      <a
        className={cn(
          'text-lg font-medium p-4 -m-4 no-underline text-gray-600 hover:text-blue-600 flex items-center',
          { 'ml-2': !isRTL, 'mr-2': isRTL }
        )}
        title={title}
      >
        {title}
        <ArrowRight
          className={cn('transform inline flex-shrink-0', {
            'rotate-180 mr-1': isRTL,
            'ml-1': !isRTL
          })}
        />
      </a>
    </Link>
  )
}

const PrevLink = ({ route, title, isRTL }) => {
  return (
    <Link href={route}>
      <a
        className={cn(
          'text-lg font-medium p-4 -m-4 no-underline text-gray-600 hover:text-blue-600 flex items-center',
          { 'mr-2': !isRTL, 'ml-2': isRTL }
        )}
        title={title}
      >
        <ArrowRight
          className={cn('transform inline flex-shrink-0', {
            'rotate-180 mr-1': !isRTL,
            'ml-1': isRTL
          })}
        />
        {title}
      </a>
    </Link>
  )
}

const createEditUrl = (repository, filepath) => {
  const repo = parseGitUrl(repository)
  if (!repo) throw new Error('Invalid `docsRepositoryBase` URL!')

  switch (repo.type) {
    case 'github':
      return `https://github.com/${repo.owner}/${repo.name}/blob/${
        repo.branch || 'main'
      }/${repo.subdir || 'pages'}${filepath}`
    case 'gitlab':
      return `https://gitlab.com/${repo.owner}/${repo.name}/-/blob/${
        repo.branch || 'master'
      }/${repo.subdir || 'pages'}${filepath}`
  }

  return '#'
}

const EditPageLink = ({ repository, text, filepath }) => {
  const url = createEditUrl(repository, filepath)
  const { locale } = useRouter()
  return (
    <a className="text-sm" href={url} target="_blank" rel="noreferrer">
      {text
        ? renderComponent(text, {
            locale
          })
        : 'Edit this page'}
    </a>
  )
}

export const NavLinks = ({ flatDirectories, currentIndex, isRTL }) => {
  const config = useConfig()
  let prev = flatDirectories[currentIndex - 1]
  let next = flatDirectories[currentIndex + 1]

  return (
    <div className="flex flex-row items-center justify-between">
      <div>
        {prev && config.prevLinks ? (
          <PrevLink route={prev.route} title={prev.title} isRTL={isRTL} />
        ) : null}
      </div>
      <div>
        {config.nextLinks && next ? (
          <NextLink route={next.route} title={next.title} isRTL={isRTL} />
        ) : null}
      </div>
    </div>
  )
}

const Footer = ({ filepathWithName, children }) => {
  const { locale } = useRouter()
  const config = useConfig()

  return (
    <footer className="mt-24">
      {children}
      <hr />
      {config.footer ? (
        <div className="mt-24 flex justify-between flex-col-reverse md:flex-row items-center md:items-end">
          <span className="text-gray-600">
            {renderComponent(config.footerText, { locale })}
          </span>
          <div className="mt-6" />
          {config.footerEditLink ? (
            <EditPageLink
              filepath={filepathWithName}
              repository={config.docsRepositoryBase}
              text={config.footerEditLink}
            />
          ) : null}
        </div>
      ) : null}
    </footer>
  )
}

export default Footer

import React from 'react'
import ArrowRight from './arrow-right'
import Link from 'next/link'
import { useRouter } from 'next/router'
import cn from 'classnames'

import renderComponent from './utils/render-component'

const NextLink = ({ route, title, isRTL }) => {
  return (
    <Link href={route}>
      <a className={cn('text-lg font-medium p-4 -m-4 no-underline text-gray-600 hover:text-blue-600 flex items-center', { 'ml-2': !isRTL, 'mr-2': isRTL })} title={title}>
        {title}
        <ArrowRight className={cn('transform inline flex-shrink-0', { 'rotate-180 mr-1': isRTL, 'ml-1': !isRTL })} />
      </a>
    </Link>
  )
}

const PrevLink = ({ route, title, isRTL }) => {
  return (
    <Link href={route}>
      <a className={cn('text-lg font-medium p-4 -m-4 no-underline text-gray-600 hover:text-blue-600 flex items-center', { 'mr-2': !isRTL, 'ml-2': isRTL })} title={title}>
        <ArrowRight className={cn('transform inline flex-shrink-0', { 'rotate-180 mr-1': !isRTL, 'ml-1': isRTL })} />
        {title}
      </a>
    </Link>
  )
}

// Make sure path is a valid url path,
// adding / in front or in the back if missing
const fixPath = path => {
  const pathWithFrontSlash = path.startsWith('/') ? path : `/${path}`
  const pathWithBackSlash = pathWithFrontSlash.endsWith('/')
    ? pathWithFrontSlash
    : `${pathWithFrontSlash}/`

  return pathWithBackSlash
}

const createEditUrl = (repository, branch, path, filepathWithName) => {
  const normalizedPath = fixPath(path)
  return `${repository}/tree/${branch}${normalizedPath}pages${filepathWithName}`
}

const EditOnGithubLink = ({
  repository,
  branch,
  path,
  footerEditOnGitHubText,
  filepathWithName
}) => {
  const href = createEditUrl(repository, branch, path, filepathWithName)
  const { locale } = useRouter()
  return (
    <a className="text-sm" href={href} target="_blank">
      {footerEditOnGitHubText
        ? renderComponent(footerEditOnGitHubText, {
            locale
          })
        : 'Edit this page on GitHub'}
    </a>
  )
}

const Footer = ({
  config,
  flatDirectories,
  currentIndex,
  filepathWithName,
  isRTL
}) => {
  let prev = flatDirectories[currentIndex - 1]
  let next = flatDirectories[currentIndex + 1]
  const { locale } = useRouter()

  return (
    <footer className="mt-24">
      <nav className="flex flex-row items-center justify-between">
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
      </nav>

      <hr />

      {config.footer ? (
        <div className="mt-24 flex justify-between flex-col-reverse md:flex-row items-center md:items-end">
          <span className="text-gray-600">
            {renderComponent(config.footerText, { locale })}
          </span>
          <div className="mt-6" />
          {config.footerEditOnGitHubLink ? (
            <EditOnGithubLink
              repository={config.docsRepository || config.repository}
              branch={config.branch}
              path={config.path}
              footerEditOnGitHubText={config.footerEditOnGitHubText}
              filepathWithName={filepathWithName}
            />
          ) : null}
        </div>
      ) : null}
    </footer>
  )
}

export default Footer

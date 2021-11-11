import React from 'react'
import cn from 'classnames'
import Slugger from 'github-slugger'
import innerText from 'react-innertext'

import { useActiveAnchor } from './misc/active-anchor'

const indent = level => {
  switch (level) {
    case 'h3':
      return { marginLeft: '1rem ' }
    case 'h4':
      return { marginLeft: '2rem ' }
    case 'h5':
      return { marginLeft: '3rem ' }
    case 'h6':
      return { marginLeft: '4rem ' }
  }
  return {}
}

export default function ToC({ titles }) {
  const slugger = new Slugger()
  const activeAnchor = useActiveAnchor()

  return (
    <div className="w-64 hidden xl:block text-sm pl-4">
      {titles ? (
        <ul className="overflow-y-auto sticky max-h-[calc(100vh-4rem)] top-16 pt-8 pb-10 m-0 list-none">
          {titles
            .filter(item => item.props.mdxType !== 'h1')
            .map(item => {
              const text = innerText(item.props.children) || ''
              const slug = slugger.slug(text)
              const state = activeAnchor[slug]

              return (
                <li key={slug} style={indent(item.props.mdxType)}>
                  <a
                    href={`#${slug}`}
                    className={cn(
                      'no-underline hover:text-gray-900 dark:hover:text-gray-100',
                      state && state.isActive
                        ? 'text-gray-900 dark:text-gray-100 font-semibold'
                        : 'text-gray-600'
                    )}
                  >
                    {text}
                  </a>
                </li>
              )
            })}
        </ul>
      ) : null}
    </div>
  )
}

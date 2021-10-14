import { Document } from '@contentful/rich-text-types'
import { Author } from './author'

export interface Post {
  slug: string
  title: string
  date: string
  coverImage: {
    url: string
    height: number
    width: number
    description: string
  }
  author: Author
  excerpt: string
  ogImage: {
    url: string
  }
  content: any
  body: string
}

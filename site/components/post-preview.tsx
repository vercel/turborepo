import Avatar from './avatar'
import DateFormatter from './date-formatter'
import CoverImage from './cover-image'
import Link from 'next/link'

import { Post } from '../types/post'

const PostPreview = ({
  title,
  coverImage,
  date,
  excerpt,
  author,
  slug,
}: Omit<Post, 'ogImage' | 'content' | 'body'>) => {
  return (
    <div>
      <h3 className="text-2xl dark:text-white font-extrabold mb-3 leading-snug">
        <Link as={`/posts/${slug}`} href="/posts/[slug]">
          <a className="hover:underline">{title}</a>
        </Link>
      </h3>
      <div className="text-lg mb-4 dark:text-gray-500">
        <DateFormatter dateString={date} />
      </div>
      <p className="text-lg prose mb-4 dark:prose-dark">{excerpt}</p>
      <Avatar
        name={author.name}
        picture={author.picture.url}
        twitterUsername={author.twitterUsername}
      />
    </div>
  )
}

export default PostPreview

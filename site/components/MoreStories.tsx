import PostPreview from './post-preview'
import { Post } from '../types/post'

interface MoreStoriesProps {
  posts: Post[]
}

export const MoreStories = ({ posts }: MoreStoriesProps) => {
  return (
    <section>
      <h1 className="mb-8 text-6xl dark:text-white font-extrabold tracking-tighter leading-tight">
        Blog
      </h1>
      <div className="grid grid-cols-1  gap-16 row-gap-20 md:row-gap-32 mb-32">
        {posts.map((post) => (
          <PostPreview
            key={post.slug}
            title={post.title}
            coverImage={post.coverImage}
            date={post.date}
            author={post.author}
            slug={post.slug}
            excerpt={post.excerpt}
          />
        ))}
      </div>
    </section>
  )
}

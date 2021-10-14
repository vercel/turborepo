import hydrate from 'next-mdx-remote/hydrate'
import renderToString from 'next-mdx-remote/render-to-string'
import { NextSeo } from 'next-seo'
import ErrorPage from 'next/error'
import { useRouter } from 'next/router'
import Avatar from '../../components/avatar'
import CodeBlock from '../../components/CodeBlock'
import { Container } from '../../components/container'
import DateFormatter from '../../components/date-formatter'
import { Header } from '../../components/Header'
import { Layout } from '../../components/Layout'
import PostTitle from '../../components/post-title'
import { getAllPostsWithSlug, getPostAndMorePosts } from '../../lib/api'
import { Post } from '../../types/post'

type Props = {
  post: Post
  next: Pick<Post, 'title' | 'slug'> | null
  prev: Pick<Post, 'title' | 'slug'> | null
  preview?: boolean
}

const Pre = (p: JSX.IntrinsicElements['div']) => <div {...p} />

const components = {
  code: CodeBlock,
  // The code block renders <pre> so we just want a div here.
  pre: Pre,
}

const PostPage = ({ post, next, prev, preview }: Props) => {
  const router = useRouter()

  if (!router.isFallback && !post?.slug) {
    return <ErrorPage statusCode={404} />
  }
  if (!post || !post?.content) {
    return (
      <Layout showCta={true} preview={preview}>
        <Container>
          <Header />

          <article className="pb-32 max-w-3xl mx-auto">
            <header className="py-6 xl:pb-10">
              <div className="space-y-6 text-center">
                <div>
                  <dl className="space-y-10">
                    <div>
                      <dt className="sr-only">Published on</dt>
                      <dd className="text-base leading-6 font-medium text-gray-500"></dd>
                    </div>
                  </dl>
                  <h1 className="text-3xl dark:text-white block  text-center leading-8 font-extrabold tracking-tight text-gray-900 sm:text-4xl lg:leading-tight lg:text-7xl "></h1>
                </div>
                <div className="mx-auto flex  justify-center"></div>
              </div>
            </header>

            <div className="prose lg:prose-lg  dark:prose-dark mx-auto max-w-3xl pb-16 xl:pb-20"></div>
          </article>
        </Container>
      </Layout>
    )
  }
  const content = hydrate(post?.content, { components })
  return (
    <Layout showCta={true} preview={preview}>
      <Container>
        <Header />

        {router.isFallback ? (
          <PostTitle>Loading…</PostTitle>
        ) : (
          <>
            <article className="pb-32 max-w-3xl mx-auto">
              <NextSeo
                title={post.title}
                description={post.excerpt}
                canonical={`https://turborepo.com/posts/${post.slug}`}
                openGraph={{
                  url: `https://turborepo.com/posts/${post.slug}`,
                  title: post.title,
                  description: post.excerpt,
                  images: [
                    {
                      url: post.coverImage.url,
                      width: post.coverImage.width,
                      height: post.coverImage.height,
                      alt: post.coverImage.description,
                    },
                  ],
                }}
              />
              <header className="py-6 xl:pb-10">
                <div className="space-y-6 text-center">
                  <div>
                    <dl className="space-y-10">
                      <div>
                        <dt className="sr-only">Published on</dt>
                        <dd className="text-base leading-6 font-medium text-gray-500">
                          <time dateTime={post.date}>
                            <DateFormatter dateString={post.date} />
                          </time>
                        </dd>
                      </div>
                    </dl>
                    <h1 className="text-3xl dark:text-white block  text-center leading-8 font-extrabold tracking-tight text-gray-900 sm:text-4xl lg:leading-tight lg:text-7xl ">
                      {post.title}
                    </h1>
                  </div>
                  <div className="mx-auto flex  justify-center">
                    <Avatar
                      name={post.author.name}
                      picture={post.author.picture.url}
                      twitterUsername={post.author.twitterUsername}
                    />
                  </div>
                </div>
              </header>

              <div className="prose lg:prose-lg dark:prose-dark mx-auto h-full relative">
                {content ?? ' '}
              </div>
            </article>
          </>
        )}
      </Container>
    </Layout>
  )
}

export default PostPage

interface Params {
  params: {
    slug: string
  }
}

export async function getStaticProps({ params, preview = false }: any) {
  const data = await getPostAndMorePosts(params.slug, preview)
  const postIndex = data?.morePosts.findIndex(
    (p: any) => p.title === data?.post.title
  )

  const maybeNextPost = data?.morePosts[postIndex + 1]
    ? data?.morePosts[postIndex + 1]
    : null
  const maybePrevPost = data?.morePosts[postIndex - 1]
    ? data?.morePosts[postIndex - 1]
    : null
  const mdxSource = await renderToString(data?.post.body, { components })

  const post = {
    ...data?.post,
    content: mdxSource,
  }
  return {
    props: {
      preview,
      post: post,
      next: maybeNextPost,
      prev: maybePrevPost,
    },
    revalidate: 60,
  }
}
export async function getStaticPaths() {
  const allPosts = await getAllPostsWithSlug()
  return {
    paths:
      allPosts?.map(({ slug }: { slug: string }) => `/posts/${slug}`) ?? [],
    fallback: true,
  }
}

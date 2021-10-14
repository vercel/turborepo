import { NextSeo } from 'next-seo'
import * as React from 'react'
import { Container } from '../components/container'
import { Header } from '../components/Header'
import { Layout } from '../components/Layout'
import { MoreStories } from '../components/MoreStories'
import { getAllPostsForHome } from '../lib/api'
import { Post } from '../types/post'

const Index = (props: { allPosts: Post[] }) => {
  return (
    <Layout showCta={true}>
      <NextSeo
        title="Blog"
        description="The latest news, announcements, and resources from Turborepo."
        openGraph={{
          url: `https://turborepo.com/blog`,
          title: 'Blog',
          description:
            'The latest news, announcements, and resources from Turborepo.',
        }}
      />
      <Container>
        <Header />
        <div className="max-w-3xl mx-auto pb-32 pt-12">
          <MoreStories posts={props.allPosts} />
        </div>
      </Container>
    </Layout>
  )
}

export const getStaticProps = async ({ preview = false }) => {
  const allPosts = (await getAllPostsForHome(preview)) ?? []

  return {
    props: { allPosts },
    revalidate: 60,
  }
}

export default Index

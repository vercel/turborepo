declare module "rss" {
  namespace RSS {
    interface FeedOptions {
      title: string;
      description: string;
      site_url: string;
      feed_url: string;
      image_url: string;
    }

    interface ItemOptions {
      title: string;
      url: string;
      date: string;
      description: string;
      enclosure?: {
        url: string;
        type: string;
        size: number;
      };
    }

    class Feed {
      constructor(options: FeedOptions);
      item(options: ItemOptions): void;
      xml(options?: { indent: boolean }): string;
    }
  }

  const RSS: typeof RSS.Feed;
  export = RSS;
}

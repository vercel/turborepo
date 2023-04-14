import { CSSProperties } from "react";

export type TurboUser = {
  caption: string;
  image: string;
  infoLink: string;
  pinned?: boolean;
  style?: CSSProperties;
};

export const users: Array<TurboUser> = [
  {
    caption: "Vercel",
    image: "/images/logos/vercel.svg",
    infoLink: "https://vercel.com/",
    pinned: true,
    style: {
      width: 150,
    },
  },
  {
    caption: "AWS",
    image: "/images/logos/aws.svg",
    infoLink: "https://aws.amazon.com/",
    pinned: true,
    style: {
      width: 75,
    },
  },
  {
    caption: "Microsoft",
    image: "/images/logos/microsoft.svg",
    infoLink: "https://www.microsoft.com/",
    pinned: true,
    style: {
      width: 125,
    },
  },
  {
    caption: "Netflix",
    image: "/images/logos/netflix.svg",
    infoLink: "https://netflix.com/",
    pinned: true,
    style: {
      width: 110,
    },
  },
  {
    caption: "Disney",
    image: "/images/logos/disney.svg",
    infoLink: "https://www.disney.com/",
    pinned: true,
  },
  {
    caption: "Github",
    image: "/images/logos/github.svg",
    infoLink: "https://www.github.com/",
    pinned: true,
    style: {
      width: 110,
    },
  },
  {
    caption: "Alibaba",
    image: "/images/logos/alibaba.svg",
    infoLink: "https://www.alibaba.com/",
    pinned: true,
    style: {
      width: 125,
    },
  },
  {
    caption: "Ant Group",
    image: "/images/logos/ant.svg",
    infoLink: "https://antgroup.com/",
    pinned: true,
    style: {
      width: 125,
    },
  },
  {
    caption: "Adobe",
    image: "/images/logos/adobe.svg",
    infoLink: "https://www.adobe.com/",
    pinned: true,
  },
  {
    caption: "PayPal",
    image: "/images/logos/paypal.svg",
    infoLink: "https://www.paypal.com/",
    pinned: true,
  },

  {
    caption: "Snap",
    image: "/images/logos/snap.svg",
    infoLink: "https://snap.com/",
    pinned: true,
  },
  {
    caption: "SAP",
    image: "/images/logos/sap.svg",
    infoLink: "https://www.sap.com/",
    pinned: true,
    style: {
      width: 75,
    },
  },

  {
    caption: "Shopify",
    image: "/images/logos/shopify.svg",
    infoLink: "https://www.shopify.com/",
    pinned: true,
    style: {
      width: 125,
    },
  },

  {
    caption: "Datadog",
    image: "/images/logos/datadog.svg",
    infoLink: "https://www.datadoghq.com/",
    pinned: true,
    style: {
      width: 125,
    },
  },
  {
    caption: "Twilio",
    image: "/images/logos/twilio.svg",
    infoLink: "https://www.twilio.com/",
    pinned: true,
  },
  {
    caption: "Segment",
    image: "/images/logos/segment.svg",
    infoLink: "https://segment.com/",
    pinned: true,
    style: {
      width: 125,
    },
  },
  {
    caption: "Twitch",
    image: "/images/logos/twitch.svg",
    infoLink: "https://www.twitch.tv/",
    pinned: true,
    style: {
      width: 125,
    },
  },
  {
    caption: "Xiaomi",
    image: "/images/logos/xiaomi.svg",
    infoLink: "https://www.mi.com/",
    pinned: true,
    style: {
      width: 50,
    },
  },
  {
    caption: "Line",
    image: "/images/logos/line.svg",
    infoLink: "https://line.me/",
    pinned: true,
    style: {
      width: 75,
    },
  },
  {
    caption: "ESPN",
    image: "/images/logos/espn.svg",
    infoLink: "https://www.espn.com/",
    pinned: true,
    style: {
      width: 125,
    },
  },
  {
    caption: "Volvo",
    image: "/images/logos/volvo.svg",
    infoLink: "https://www.volvo.com/",
    pinned: true,
    style: {
      width: 60,
    },
  },
  {
    caption: "Hearst",
    image: "/images/logos/hearst.svg",
    infoLink: "https://www.hearst.com/",
    pinned: true,
    style: {
      width: 175,
    },
  },
  {
    caption: "The Washington Post",
    image: "/images/logos/washingtonpost.svg",
    infoLink: "https://www.washingtonpost.com/",
    pinned: true,
    style: {
      width: 175,
    },
  },
  {
    caption: "Wayfair",
    image: "/images/logos/wayfair.svg",
    infoLink: "https://www.wayfair.com/",
    pinned: true,
    style: {
      width: 125,
    },
  },
  {
    caption: "Hulu",
    image: "/images/logos/hulu.svg",
    infoLink: "https://www.hulu.com/",
    pinned: true,
  },
  {
    caption: "CrowdStrike",
    image: "/images/logos/crowdstrike.svg",
    infoLink: "https://www.crowdstrike.com/",
    pinned: true,
    style: {
      width: 150,
      marginTop: 20,
    },
  },
  {
    caption: "Binance",
    image: "/images/logos/binance.svg",
    infoLink: "https://www.binance.com/",
    pinned: true,
    style: {
      width: 150,
    },
  },
  {
    caption: "Plex",
    image: "/images/logos/plex.svg",
    infoLink: "https://www.plex.tv/",
    pinned: true,
  },
  {
    caption: "Groupon",
    image: "/images/logos/groupon.svg",
    infoLink: "https://groupon.com/",
    pinned: true,
    style: {
      width: 125,
    },
  },
  {
    caption: "Vimeo",
    image: "/images/logos/vimeo.svg",
    infoLink: "https://vimeo.com/",
    pinned: true,
  },
  {
    caption: "GoodRx",
    image: "/images/logos/goodrx.svg",
    infoLink: "https://www.goodrx.com/",
    pinned: true,
    style: {
      width: 150,
    },
  },
  {
    caption: "Tripadvisor",
    image: "/images/logos/tripadvisor.svg",
    infoLink: "https://www.tripadvisor.com/",
    pinned: true,
    style: {
      width: 150,
    },
  },
  {
    caption: "RapidAPI",
    image: "/images/logos/rapidapi.svg",
    infoLink: "https://rapidapi.com/",
    pinned: true,
    style: {
      width: 150,
    },
  },
  {
    caption: "Miro",
    image: "/images/logos/miro.svg",
    infoLink: "https://miro.com/",
    pinned: true,
  },
  {
    caption: "Lattice",
    image: "/images/logos/lattice.svg",
    infoLink: "https://lattice.com/",
    pinned: true,
    style: {
      width: 150,
    },
  },
  {
    caption: "Watershed",
    image: "/images/logos/watershed.svg",
    infoLink: "https://watershed.com/",
    pinned: true,
    style: {
      width: 150,
    },
  },
  {
    caption: "N26",
    image: "/images/logos/n26.svg",
    infoLink: "https://n26.com/",
    pinned: true,
    style: {
      width: 75,
    },
  },
  {
    caption: "Sourcegraph",
    image: "/images/logos/sourcegraph.svg",
    infoLink: "https://sourcegraph.com/",
    pinned: true,
    style: {
      width: 150,
    },
  },
  {
    caption: "Big Commerce",
    image: "/images/logos/bigcommerce.svg",
    infoLink: "https://www.bigcommerce.com/",
    pinned: true,
    style: {
      width: 150,
    },
  },
  {
    caption: "Stedi",
    image: "/images/logos/stedi.svg",
    infoLink: "https://www.stedi.com/",
    pinned: true,
    style: {
      width: 75,
    },
  },
  {
    caption: "Framer",
    image: "/images/logos/framer.svg",
    infoLink: "https://www.framer.com/",
    pinned: true,
  },
  {
    caption: "Maze",
    image: "/images/logos/maze.svg",
    infoLink: "https://maze.co/",
    pinned: true,
    style: {
      width: 125,
    },
  },
  {
    caption: "Builder.io",
    image: "/images/logos/builderio.svg",
    infoLink: "https://www.builder.io/",
    pinned: true,
    style: {
      width: 125,
    },
  },
  {
    caption: "Contentful",
    image: "/images/logos/contentful.svg",
    infoLink: "https://www.contentful.com/",
    pinned: true,
    style: {
      width: 150,
    },
  },
  {
    caption: "Xata",
    image: "/images/logos/xata.svg",
    infoLink: "https://xata.io/",
    pinned: true,
  },
  {
    caption: "Cal.com",
    image: "/images/logos/calcom.svg",
    infoLink: "https://cal.com/",
    pinned: true,
  },
  {
    caption: "Codesandbox",
    image: "/images/logos/codesandbox.svg",
    infoLink: "https://codesandbox.io/",
    pinned: true,
    style: {
      width: 150,
    },
  },
  {
    caption: "WooCommerce",
    image: "/images/logos/woocommerce.svg",
    infoLink: "https://woocommerce.com/",
    pinned: true,
    style: {
      width: 150,
    },
  },
  {
    caption: "Expo",
    image: "/images/logos/expo.svg",
    infoLink: "https://expo.dev/",
    pinned: true,
  },
  {
    caption: "TeeSpring",
    image: "/images/logos/teespring.svg",
    infoLink: "https://www.spri.ng/",
    pinned: true,
    style: {
      width: 150,
    },
  },
  {
    caption: "Endear",
    image: "/images/logos/endear.svg",
    infoLink: "https://endearhq.com/",
    pinned: true,
  },
  {
    caption: "Makeswift",
    image: "/images/logos/makeswift.svg",
    infoLink: "https://www.makeswift.com/",
    pinned: true,
  },
  {
    caption: "Fandom",
    image: "/images/logos/fandom.svg",
    infoLink: "https://www.fandom.com/",
    pinned: true,
  },
  {
    caption: "Waggel",
    image: "/images/logos/waggel.svg",
    infoLink: "https://www.waggel.co.uk/",
    pinned: true,
  },
  {
    caption: "n8n",
    image: "/images/logos/n8n.svg",
    infoLink: "https://n8n.io/",
    pinned: true,
  },
  {
    caption: "React Flow",
    image: "/images/logos/reactflow.svg",
    infoLink: "https://reactflow.dev/",
    pinned: true,
  },
  {
    caption: "Agrotoken",
    image: "/images/logos/agrotoken.svg",
    infoLink: "https://agrotoken.io/",
    pinned: true,
  },
  {
    caption: "Rocket.Chat",
    image: "/images/logos/rocketchat.svg",
    infoLink: "https://www.rocket.chat/",
    pinned: true,
  },
  {
    caption: "Backpack",
    image: "/images/logos/backpack.svg",
    infoLink: "https://www.backpack.app/",
    pinned: true,
  },
  {
    caption: "Supernova.io",
    image: "/images/logos/supernova.svg",
    infoLink: "https://www.supernova.io/",
    pinned: true,
    style: {
      width: 150,
    },
  },
  {
    caption: "BuildPass",
    image: "/images/logos/buildpass.svg",
    infoLink: "https://www.buildpass.com.au/",
    pinned: true,
    style: {
      width: 175,
    },
  },
  {
    caption: "SolanaFM",
    image: "/images/logos/solanafm.svg",
    infoLink: "https://solana.fm/",
    pinned: true,
  },
  {
    caption: "Authdog",
    image: "/images/logos/authdog.svg",
    infoLink: "https://www.authdog.com/",
    pinned: true,
    style: {
      width: 175,
    },
  },
  {
    caption: "Nhost",
    image: "/images/logos/nhost.svg",
    infoLink: "https://nhost.io/",
    pinned: true,
  },
  {
    caption: "LG U+",
    image: "/images/logos/lguplus.svg",
    infoLink: "https://www.lguplus.com/about/en",
    pinned: true,
    style: {
      width: 150,
    },
  },
  {
    caption: "Comparastore",
    image: "/images/logos/comparastore.svg",
    infoLink: "https://www.comparastore.com",
    pinned: true,
    style: {
      width: 175,
    },
  },
  {
    caption: "Block Protocol",
    image: "/images/logos/blockprotocol.svg",
    infoLink: "https://blockprotocol.org/",
    pinned: true,
  },
  {
    caption: "HASH",
    image: "/images/logos/hash.svg",
    infoLink: "https://hash.dev/",
    pinned: true,
  },
  {
    caption: "Chợ Tốt",
    image: "/images/logos/chotot.svg",
    infoLink: "https://chotot.com/",
    pinned: true,
  },
];

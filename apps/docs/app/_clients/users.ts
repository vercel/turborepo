import type { CSSProperties } from "react";
import type { StaticImageData } from "next/image";

// White logos (for dark mode)
import adobeWhite from "../../public/images/logos/white/adobe.svg";
import alibabaWhite from "../../public/images/logos/white/alibaba.svg";
import antWhite from "../../public/images/logos/white/ant.svg";
import awsWhite from "../../public/images/logos/white/aws.svg";
import bigcommerceWhite from "../../public/images/logos/white/bigcommerce.svg";
import binanceWhite from "../../public/images/logos/white/binance.svg";
import builderioWhite from "../../public/images/logos/white/builderio.svg";
import calcomWhite from "../../public/images/logos/white/calcom.svg";
import codesandboxWhite from "../../public/images/logos/white/codesandbox.svg";
import contentfulWhite from "../../public/images/logos/white/contentful.svg";
import crowdstrikeWhite from "../../public/images/logos/white/crowdstrike.svg";
import datadogWhite from "../../public/images/logos/white/datadog.svg";
import disneyWhite from "../../public/images/logos/white/disney.svg";
import endearWhite from "../../public/images/logos/white/endear.svg";
import espnWhite from "../../public/images/logos/white/espn.svg";
import expoWhite from "../../public/images/logos/white/expo.svg";
import framerWhite from "../../public/images/logos/white/framer.svg";
import githubWhite from "../../public/images/logos/white/github.svg";
import goodrxWhite from "../../public/images/logos/white/goodrx.svg";
import grouponWhite from "../../public/images/logos/white/groupon.svg";
import hearstWhite from "../../public/images/logos/white/hearst.svg";
import huluWhite from "../../public/images/logos/white/hulu.svg";
import latticeWhite from "../../public/images/logos/white/lattice.svg";
import lineWhite from "../../public/images/logos/white/line.svg";
import linearWhite from "../../public/images/logos/white/linear.svg";
import makeswiftWhite from "../../public/images/logos/white/makeswift.svg";
import microsoftWhite from "../../public/images/logos/white/microsoft.svg";
import miroWhite from "../../public/images/logos/white/miro.svg";
import n26White from "../../public/images/logos/white/n26.svg";
import netflixWhite from "../../public/images/logos/white/netflix.svg";
import paypalWhite from "../../public/images/logos/white/paypal.svg";
import plexWhite from "../../public/images/logos/white/plex.svg";
import rapidapiWhite from "../../public/images/logos/white/rapidapi.svg";
import sapWhite from "../../public/images/logos/white/sap.svg";
import segmentWhite from "../../public/images/logos/white/segment.svg";
import shopifyWhite from "../../public/images/logos/white/shopify.svg";
import snapWhite from "../../public/images/logos/white/snap.svg";
import sourcegraphWhite from "../../public/images/logos/white/sourcegraph.svg";
import tripadvisorWhite from "../../public/images/logos/white/tripadvisor.svg";
import twilioWhite from "../../public/images/logos/white/twilio.svg";
import twitchWhite from "../../public/images/logos/white/twitch.svg";
import vercelWhite from "../../public/images/logos/white/vercel.svg";
import vimeoWhite from "../../public/images/logos/white/vimeo.svg";
import volvoWhite from "../../public/images/logos/white/volvo.svg";
import washingtonpostWhite from "../../public/images/logos/white/washingtonpost.svg";
import watershedWhite from "../../public/images/logos/white/watershed.svg";
import wayfairWhite from "../../public/images/logos/white/wayfair.svg";
import woocommerceWhite from "../../public/images/logos/white/woocommerce.svg";
import xataWhite from "../../public/images/logos/white/xata.svg";
import xiaomiWhite from "../../public/images/logos/white/xiaomi.svg";

// Color logos (for light mode)
import adobeColor from "../../public/images/logos/color/adobe.svg";
import alibabaColor from "../../public/images/logos/color/alibaba.svg";
import antColor from "../../public/images/logos/color/ant.svg";
import awsColor from "../../public/images/logos/color/aws.svg";
import bigcommerceColor from "../../public/images/logos/color/bigcommerce.svg";
import binanceColor from "../../public/images/logos/color/binance.svg";
import builderioColor from "../../public/images/logos/color/builderio.svg";
import calcomColor from "../../public/images/logos/color/calcom.svg";
import codesandboxColor from "../../public/images/logos/color/codesandbox.svg";
import contentfulColor from "../../public/images/logos/color/contentful.svg";
import crowdstrikeColor from "../../public/images/logos/color/crowdstrike.svg";
import datadogColor from "../../public/images/logos/color/datadog.svg";
import disneyColor from "../../public/images/logos/color/disney.svg";
import endearColor from "../../public/images/logos/color/endear.svg";
import espnColor from "../../public/images/logos/color/espn.svg";
import expoColor from "../../public/images/logos/color/expo.svg";
import framerColor from "../../public/images/logos/color/framer.svg";
import githubColor from "../../public/images/logos/color/github.svg";
import goodrxColor from "../../public/images/logos/color/goodrx.svg";
import grouponColor from "../../public/images/logos/color/groupon.svg";
import hearstColor from "../../public/images/logos/color/hearst.svg";
import huluColor from "../../public/images/logos/color/hulu.svg";
import latticeColor from "../../public/images/logos/color/lattice.svg";
import lineColor from "../../public/images/logos/color/line.svg";
import linearColor from "../../public/images/logos/color/linear.svg";
import makeswiftColor from "../../public/images/logos/color/makeswift.svg";
import microsoftColor from "../../public/images/logos/color/microsoft.svg";
import miroColor from "../../public/images/logos/color/miro.svg";
import n26Color from "../../public/images/logos/color/n26.svg";
import netflixColor from "../../public/images/logos/color/netflix.svg";
import paypalColor from "../../public/images/logos/color/paypal.svg";
import plexColor from "../../public/images/logos/color/plex.svg";
import rapidapiColor from "../../public/images/logos/color/rapidapi.svg";
import sapColor from "../../public/images/logos/color/sap.svg";
import segmentColor from "../../public/images/logos/color/segment.svg";
import shopifyColor from "../../public/images/logos/color/shopify.svg";
import snapColor from "../../public/images/logos/color/snap.svg";
import sourcegraphColor from "../../public/images/logos/color/sourcegraph.svg";
import tripadvisorColor from "../../public/images/logos/color/tripadvisor.svg";
import twilioColor from "../../public/images/logos/color/twilio.svg";
import twitchColor from "../../public/images/logos/color/twitch.svg";
import vercelColor from "../../public/images/logos/color/vercel.svg";
import vimeoColor from "../../public/images/logos/color/vimeo.svg";
import volvoColor from "../../public/images/logos/color/volvo.svg";
import washingtonpostColor from "../../public/images/logos/color/washingtonpost.svg";
import watershedColor from "../../public/images/logos/color/watershed.svg";
import wayfairColor from "../../public/images/logos/color/wayfair.svg";
import woocommerceColor from "../../public/images/logos/color/woocommerce.svg";
import xataColor from "../../public/images/logos/color/xata.svg";
import xiaomiColor from "../../public/images/logos/color/xiaomi.svg";

export interface TurboUser {
  caption: string;
  imageWhite: StaticImageData;
  imageColor: StaticImageData;
  infoLink: string;
  pinned?: boolean;
  style?: CSSProperties;
}

export const users: Array<TurboUser> = [
  {
    caption: "Vercel",
    imageWhite: vercelWhite,
    imageColor: vercelColor,
    infoLink: "https://vercel.com/",
    pinned: true,
    style: {
      width: 150
    }
  },
  {
    caption: "AWS",
    imageWhite: awsWhite,
    imageColor: awsColor,
    infoLink: "https://aws.amazon.com/",
    pinned: true,
    style: {
      width: 75
    }
  },
  {
    caption: "Microsoft",
    imageWhite: microsoftWhite,
    imageColor: microsoftColor,
    infoLink: "https://www.microsoft.com/",
    pinned: true,
    style: {
      width: 125
    }
  },
  {
    caption: "Netflix",
    imageWhite: netflixWhite,
    imageColor: netflixColor,
    infoLink: "https://netflix.com/",
    pinned: true,
    style: {
      width: 110
    }
  },
  {
    caption: "Disney",
    imageWhite: disneyWhite,
    imageColor: disneyColor,
    infoLink: "https://www.disney.com/",
    pinned: true
  },
  {
    caption: "Github",
    imageWhite: githubWhite,
    imageColor: githubColor,
    infoLink: "https://www.github.com/",
    pinned: true,
    style: {
      width: 110
    }
  },
  {
    caption: "Linear",
    imageWhite: linearWhite,
    imageColor: linearColor,
    infoLink: "https://www.linear.app/",
    pinned: true,
    style: {
      width: 110
    }
  },
  {
    caption: "Alibaba",
    imageWhite: alibabaWhite,
    imageColor: alibabaColor,
    infoLink: "https://www.alibaba.com/",
    pinned: true,
    style: {
      width: 125
    }
  },
  {
    caption: "Ant Group",
    imageWhite: antWhite,
    imageColor: antColor,
    infoLink: "https://antgroup.com/",
    pinned: true,
    style: {
      width: 125
    }
  },
  {
    caption: "Adobe",
    imageWhite: adobeWhite,
    imageColor: adobeColor,
    infoLink: "https://www.adobe.com/",
    pinned: true
  },
  {
    caption: "PayPal",
    imageWhite: paypalWhite,
    imageColor: paypalColor,
    infoLink: "https://www.paypal.com/",
    pinned: true
  },
  {
    caption: "Snap",
    imageWhite: snapWhite,
    imageColor: snapColor,
    infoLink: "https://snap.com/",
    pinned: true
  },
  {
    caption: "SAP",
    imageWhite: sapWhite,
    imageColor: sapColor,
    infoLink: "https://www.sap.com/",
    pinned: true,
    style: {
      width: 75
    }
  },
  {
    caption: "Shopify",
    imageWhite: shopifyWhite,
    imageColor: shopifyColor,
    infoLink: "https://www.shopify.com/",
    pinned: true,
    style: {
      width: 125
    }
  },
  {
    caption: "Datadog",
    imageWhite: datadogWhite,
    imageColor: datadogColor,
    infoLink: "https://www.datadoghq.com/",
    pinned: true,
    style: {
      width: 125
    }
  },
  {
    caption: "Twilio",
    imageWhite: twilioWhite,
    imageColor: twilioColor,
    infoLink: "https://www.twilio.com/",
    pinned: true
  },
  {
    caption: "Segment",
    imageWhite: segmentWhite,
    imageColor: segmentColor,
    infoLink: "https://segment.com/",
    pinned: true,
    style: {
      width: 125
    }
  },
  {
    caption: "Twitch",
    imageWhite: twitchWhite,
    imageColor: twitchColor,
    infoLink: "https://www.twitch.tv/",
    pinned: true,
    style: {
      width: 125
    }
  },
  {
    caption: "Xiaomi",
    imageWhite: xiaomiWhite,
    imageColor: xiaomiColor,
    infoLink: "https://www.mi.com/",
    pinned: true,
    style: {
      width: 50
    }
  },
  {
    caption: "Line",
    imageWhite: lineWhite,
    imageColor: lineColor,
    infoLink: "https://line.me/",
    pinned: true,
    style: {
      width: 75
    }
  },
  {
    caption: "ESPN",
    imageWhite: espnWhite,
    imageColor: espnColor,
    infoLink: "https://www.espn.com/",
    pinned: true,
    style: {
      width: 125
    }
  },
  {
    caption: "Volvo",
    imageWhite: volvoWhite,
    imageColor: volvoColor,
    infoLink: "https://www.volvo.com/",
    pinned: true,
    style: {
      width: 60
    }
  },
  {
    caption: "Hearst",
    imageWhite: hearstWhite,
    imageColor: hearstColor,
    infoLink: "https://www.hearst.com/",
    pinned: true,
    style: {
      width: 175
    }
  },
  {
    caption: "The Washington Post",
    imageWhite: washingtonpostWhite,
    imageColor: washingtonpostColor,
    infoLink: "https://www.washingtonpost.com/",
    pinned: true,
    style: {
      width: 175
    }
  },
  {
    caption: "Wayfair",
    imageWhite: wayfairWhite,
    imageColor: wayfairColor,
    infoLink: "https://www.wayfair.com/",
    pinned: true,
    style: {
      width: 125
    }
  },
  {
    caption: "Hulu",
    imageWhite: huluWhite,
    imageColor: huluColor,
    infoLink: "https://www.hulu.com/",
    pinned: true
  },
  {
    caption: "CrowdStrike",
    imageWhite: crowdstrikeWhite,
    imageColor: crowdstrikeColor,
    infoLink: "https://www.crowdstrike.com/",
    pinned: true,
    style: {
      width: 150,
      marginTop: 20
    }
  },
  {
    caption: "Binance",
    imageWhite: binanceWhite,
    imageColor: binanceColor,
    infoLink: "https://www.binance.com/",
    pinned: true,
    style: {
      width: 150
    }
  },
  {
    caption: "Plex",
    imageWhite: plexWhite,
    imageColor: plexColor,
    infoLink: "https://www.plex.tv/",
    pinned: true
  },
  {
    caption: "Groupon",
    imageWhite: grouponWhite,
    imageColor: grouponColor,
    infoLink: "https://groupon.com/",
    pinned: true,
    style: {
      width: 125
    }
  },
  {
    caption: "Vimeo",
    imageWhite: vimeoWhite,
    imageColor: vimeoColor,
    infoLink: "https://vimeo.com/",
    pinned: true
  },
  {
    caption: "GoodRx",
    imageWhite: goodrxWhite,
    imageColor: goodrxColor,
    infoLink: "https://www.goodrx.com/",
    pinned: true,
    style: {
      width: 150
    }
  },
  {
    caption: "Tripadvisor",
    imageWhite: tripadvisorWhite,
    imageColor: tripadvisorColor,
    infoLink: "https://www.tripadvisor.com/",
    pinned: true,
    style: {
      width: 150
    }
  },
  {
    caption: "RapidAPI",
    imageWhite: rapidapiWhite,
    imageColor: rapidapiColor,
    infoLink: "https://rapidapi.com/",
    pinned: true,
    style: {
      width: 150
    }
  },
  {
    caption: "Miro",
    imageWhite: miroWhite,
    imageColor: miroColor,
    infoLink: "https://miro.com/",
    pinned: true
  },
  {
    caption: "Lattice",
    imageWhite: latticeWhite,
    imageColor: latticeColor,
    infoLink: "https://lattice.com/",
    pinned: true,
    style: {
      width: 150
    }
  },
  {
    caption: "Watershed",
    imageWhite: watershedWhite,
    imageColor: watershedColor,
    infoLink: "https://watershed.com/",
    pinned: true,
    style: {
      width: 150
    }
  },
  {
    caption: "N26",
    imageWhite: n26White,
    imageColor: n26Color,
    infoLink: "https://n26.com/",
    pinned: true,
    style: {
      width: 75
    }
  },
  {
    caption: "Sourcegraph",
    imageWhite: sourcegraphWhite,
    imageColor: sourcegraphColor,
    infoLink: "https://sourcegraph.com/",
    pinned: true,
    style: {
      width: 150
    }
  },
  {
    caption: "Big Commerce",
    imageWhite: bigcommerceWhite,
    imageColor: bigcommerceColor,
    infoLink: "https://www.bigcommerce.com/",
    pinned: true,
    style: {
      width: 150
    }
  },
  {
    caption: "Framer",
    imageWhite: framerWhite,
    imageColor: framerColor,
    infoLink: "https://www.framer.com/",
    pinned: true
  },
  {
    caption: "Builder.io",
    imageWhite: builderioWhite,
    imageColor: builderioColor,
    infoLink: "https://www.builder.io/",
    pinned: true,
    style: {
      width: 125
    }
  },
  {
    caption: "Contentful",
    imageWhite: contentfulWhite,
    imageColor: contentfulColor,
    infoLink: "https://www.contentful.com/",
    pinned: true,
    style: {
      width: 150
    }
  },
  {
    caption: "Xata",
    imageWhite: xataWhite,
    imageColor: xataColor,
    infoLink: "https://xata.io/",
    pinned: true
  },
  {
    caption: "Cal.com",
    imageWhite: calcomWhite,
    imageColor: calcomColor,
    infoLink: "https://cal.com/",
    pinned: true
  },
  {
    caption: "Codesandbox",
    imageWhite: codesandboxWhite,
    imageColor: codesandboxColor,
    infoLink: "https://codesandbox.io/",
    pinned: true,
    style: {
      width: 150
    }
  },
  {
    caption: "WooCommerce",
    imageWhite: woocommerceWhite,
    imageColor: woocommerceColor,
    infoLink: "https://woocommerce.com/",
    pinned: true,
    style: {
      width: 150
    }
  },
  {
    caption: "Expo",
    imageWhite: expoWhite,
    imageColor: expoColor,
    infoLink: "https://expo.dev/",
    pinned: true
  },
  {
    caption: "Endear",
    imageWhite: endearWhite,
    imageColor: endearColor,
    infoLink: "https://endearhq.com/",
    pinned: true
  },
  {
    caption: "Makeswift",
    imageWhite: makeswiftWhite,
    imageColor: makeswiftColor,
    infoLink: "https://www.makeswift.com/",
    pinned: true
  }
];

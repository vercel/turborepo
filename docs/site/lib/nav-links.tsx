import {
  BookOpenIcon,
  ExternalLinkIcon,
  StarIcon,
} from "@heroicons/react/outline";
import type { LinkItemType } from "fumadocs-ui/layouts/links";

export const navLinks: LinkItemType[] = [
  { url: "/blog", text: "Blog", icon: <BookOpenIcon /> },
  { url: "/showcase", text: "Showcase", icon: <StarIcon /> },
  {
    url: "https://vercel.com/contact/sales?utm_source=turbo.build&utm_medium=referral&utm_campaign=header-enterpriseLink",
    text: "Enterprise",
    icon: <ExternalLinkIcon />,
  },
];

import { Heading } from "nextra";

export default function getHeadingText(heading: Heading) {
  return heading.value || "";
}

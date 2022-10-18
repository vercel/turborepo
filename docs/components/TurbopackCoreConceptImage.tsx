import Image from "next/future/image";
import firstRun from "../images/turbo-engine-first-run.png";
import secondRun from "../images/turbo-engine-second-run.png";

export function TurbopackCoreConceptImage(props: {
  img: "first" | "second";
  alt: string;
}) {
  return (
    <Image
      src={props.img === "first" ? firstRun : secondRun}
      alt={props.alt}
      className="max-w-xl my-6"
    ></Image>
  );
}

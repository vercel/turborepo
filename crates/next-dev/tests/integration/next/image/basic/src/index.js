import Image from 'next/image'
import img from '../public/triangle-black.png';

export default function Home() {
  return [
    <Image alt="test imported image" src={img} width="100" height="100" />,
    <Image alt="test src image" src="/triangle-black.png" width="100" height="100" />,
  ];
}

// TODO: assertions

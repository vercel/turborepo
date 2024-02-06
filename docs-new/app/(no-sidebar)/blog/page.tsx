import { blogPageTree } from "@/app/source";

const Page = () => {
  return <pre>{JSON.stringify(blogPageTree, null, 2)}</pre>;
};

export default Page;

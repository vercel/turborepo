import { blogPageTree, blogFiles } from "@/app/source";

const Page = () => {
  return <pre>{JSON.stringify(blogFiles, null, 2)}</pre>;
};

export default Page;

import { api } from "@/lib/eden";

export default async function Page() {
  const { data, error } = await api.get();

  if (error) {
    return <div>Error: {String(error.value)}</div>;
  }

  return <h1>{data}</h1>;
}

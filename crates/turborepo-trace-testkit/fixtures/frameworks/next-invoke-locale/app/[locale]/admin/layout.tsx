import { loadAdmin } from '../../../lib/loaders';

// The gate of every admin page: the loader redirects whoever is not an administrator.
export default async function AdminLayout({ children }: { children: React.ReactNode }) {
  const user = await loadAdmin();
  return (
    <section>
      <header>{user.name}</header>
      {children}
    </section>
  );
}

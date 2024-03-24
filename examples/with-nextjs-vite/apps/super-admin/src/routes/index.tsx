import HomeLayout from '@/Layout';
import { coinsRoute } from '@/pages/Home';
import { loginRoute } from '@/pages/Login';
import { QueryCache, QueryClient } from '@tanstack/react-query';
import {
  Link,
  Outlet,
  RouterProvider,
  createRootRouteWithContext,
  createRoute,
  createRouter,
  redirect,
} from '@tanstack/react-router';
import { TanStackRouterDevtools } from '@tanstack/router-devtools';
import { Layout, Result, Spin } from 'antd';
import modal from 'antd/es/modal';
import { lazy } from 'react';
import ReactJson from 'react-json-view';
import { Auth, auth } from './types';
import NotFound from '@/pages/NotFound';

const Home = lazy(() => import('@/pages/Home'));
const AppHeader = lazy(() => import('@/components/AppHeader'));
const AppSider = lazy(() => import('@/components/AppSideBar'));

// Build our routes. We could do this in our component, too.
export const rootRoute = createRootRouteWithContext<{
  auth: Auth;
  queryClient: QueryClient;
}>()({
  wrapInSuspense: true,
  notFoundComponent(props) {
    return <NotFound />;
  },
  component: RootComponent,
});

function RootComponent() {
  return (
    <Layout style={{ height: '100vh' }}>
      <AppSider />
      <Layout className="site-layout">
        <AppHeader />
        <HomeLayout>
          <Outlet />
        </HomeLayout>
      </Layout>
      <TanStackRouterDevtools />
    </Layout>
  );
}

export const authRoute = createRoute({
  getParentRoute: () => rootRoute,
  id: 'auth',
  // Before loading, authenticate the user via our auth context
  // This will also happen during prefetching (e.g. hovering over links, etc)
  beforeLoad: ({ context, location }) => {
    // If the user is logged out, redirect them to the login page
    if (context.auth.status === 'loggedOut') {
      throw redirect({
        to: loginRoute.to,
      });
    }

    // Otherwise, return the user in context
    return {
      username: auth.username,
    };
  },
});

const indexRoute = createRoute({
  getParentRoute: () => authRoute,
  path: '/',
  component: Home,
});

const routeTree = rootRoute.addChildren([
  authRoute.addChildren([indexRoute, coinsRoute]),
  loginRoute,
]);
// Create a new router instance
// const router = createRouter({ routeTree });
const router = createRouter({
  routeTree,
  defaultPendingComponent: () => (
    <div className={`mx-auto p-2 text-2xl`}>
      <Spin />
    </div>
  ),
  defaultErrorComponent: ({ error }) => (
    <Result
      status="warning"
      title="There are some problems with your operation."
      extra={
        <Link type="primary" key="console" href="/">
          Go Console
        </Link>
      }
    />
  ),
  context: {
    auth: undefined!, // We'll inject this when we render
    queryClient: undefined!,
  },
  defaultPreload: 'intent',
  // Since we're using React Query, we don't want loader calls to ever be stale
  // This will ensure that the loader is always called when the route is preloaded or visited
  defaultPreloadStaleTime: 0,
});
// Register the router instance for type safety
declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router;
  }
}

// route name: bread crumb name

const AppRouter = () => {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        refetchOnWindowFocus: false, // default: true
        staleTime: 60 * 1000,
      },
      mutations: {
        onError(err: any) {
          console.log(`[MUTATION ERROR]: ${err}`);
          modal.error({
            title: 'Encountered an Error',
            content: <ReactJson src={err} theme="monokai" />,
            // onOk: () => navigate('/'),
            width: '60vh',
          });
        },
      },
    },
    queryCache: new QueryCache({
      onError: (error, query) => {
        if (query.state.data !== undefined) {
          console.error(`[CACHE ERROR]: ${error}`);
        }
      },
    }),
  });
  return (
    <>
      <RouterProvider
        router={router}
        defaultPreload="intent"
        context={{
          auth,
          queryClient: queryClient,
        }}
      />
    </>
  );
};

export default AppRouter;

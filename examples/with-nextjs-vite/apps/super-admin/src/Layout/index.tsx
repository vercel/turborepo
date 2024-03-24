import AppHeader from '@/components/AppHeader';
import { userAuth } from '@/store';
import { Outlet } from '@tanstack/react-router';
import { Skeleton, theme } from 'antd';
import ErrorBoundary from 'antd/es/alert/ErrorBoundary';
import { Content } from 'antd/es/layout/layout';
import { useAtom } from 'jotai';
import { Suspense, useState } from 'react';

function HomeLayout({ children }: { children: React.ReactNode }) {
  const {
    token: { colorBgContainer },
  } = theme.useToken();

  return (
    <Content
      style={{
        margin: '24px 16px',
        padding: 24,
        minHeight: 280,
        background: colorBgContainer,
        overflow: 'scroll',
      }}
    >
      <ErrorBoundary>
        <Suspense fallback={<Skeleton active={true} />}>{children}</Suspense>
      </ErrorBoundary>
    </Content>
  );
}

export default HomeLayout;

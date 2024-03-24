import { rootRoute } from '@/routes';
import { collapsedAtom } from '@/store';
import { BarChartOutlined } from '@ant-design/icons';
import { RootRoute, useNavigate } from '@tanstack/react-router';
import { Menu, Typography, theme } from 'antd';
import Sider from 'antd/es/layout/Sider';
import { useAtom } from 'jotai';
import { useMemo } from 'react';

const { Text } = Typography;

function AppSider() {
  const {
    token: { colorFill },
  } = theme.useToken();

  const [collapsed, setCollapsed] = useAtom(collapsedAtom);

  const navigate = useNavigate({ from: '/' });

  const { auth, status } = rootRoute.useRouteContext({
    select: ({ auth }) => ({ auth, status: auth.status }),
  });

  // const location = useLocation();

  const pathSnippets = useMemo(
    () => window.location.pathname.split('/').filter((i) => i),
    [location],
  );

  function onClick(key: string) {
    navigate({
      to: key,
    });
  }

  if (auth.status === 'loggedOut') {
    return null;
  }

  return (
    <Sider trigger={null} collapsible collapsed={collapsed} style={{ background: colorFill }}>
      <span className="text-1xl py-9 text-white">Su Admin</span>
      <Menu
        style={{ background: colorFill }}
        theme="dark"
        mode="inline"
        defaultSelectedKeys={[`/${pathSnippets}`]}
        // selectedKeys={[`${selectedKeys[0]}`]}
        onClick={(e) => onClick(e.key)}
        items={[
          {
            key: '/',
            icon: <BarChartOutlined />,
            label: 'Home',
          },
        ]}
      />
    </Sider>
  );
}

export default AppSider;

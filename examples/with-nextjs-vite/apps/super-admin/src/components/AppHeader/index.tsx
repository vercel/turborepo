import { rootRoute } from '@/routes';
import { collapsedAtom } from '@/store';
import {
  LogoutOutlined,
  MenuFoldOutlined,
  MenuUnfoldOutlined,
  UserOutlined,
} from '@ant-design/icons';
import { App, Button, Dropdown, MenuProps, Typography, theme } from 'antd';
import Avatar from 'antd/es/avatar/avatar';
import { Header } from 'antd/es/layout/layout';
import { useAtom } from 'jotai';
import { createElement, useState } from 'react';
import packageJson from '../../../package.json';
import './index.css';

const { Text } = Typography;

function AppHeader() {
  const {
    token: { colorFill },
  } = theme.useToken();

  const { auth, status } = rootRoute.useRouteContext({
    select: ({ auth }) => ({ auth, status: auth.status }),
  });

  const [loading, setLoading] = useState(false);
  const [collapsed, setCollapsed] = useAtom(collapsedAtom);

  const items: MenuProps['items'] = [
    {
      key: 'signOut',
      label: (
        <Button loading={loading} disabled={status == 'loggedOut'} danger icon={<LogoutOutlined />}>
          Sign Out
        </Button>
      ),
      disabled: true,
    },
    {
      key: 'version',
      label: (
        <div style={{ width: '100%', textAlign: 'center' }}>Version: {packageJson.version}</div>
      ),
      disabled: true,
    },
  ];

  return (
    <Header className="main-header" style={{ paddingLeft: 0, background: colorFill }}>
      {createElement(collapsed ? MenuUnfoldOutlined : MenuFoldOutlined, {
        className: 'trigger',
        onClick: () => setCollapsed(!collapsed),
      })}
      <div>
        <Text strong>{auth.username ?? 'Login'}</Text>

        <Dropdown menu={{ items }} trigger={['click']} placement="bottomLeft">
          <a onClick={(e) => e.preventDefault()}>
            <Avatar style={{ backgroundColor: '#87d068' }} icon={<UserOutlined />} />
          </a>
        </Dropdown>
      </div>
    </Header>
  );
}

export default AppHeader;

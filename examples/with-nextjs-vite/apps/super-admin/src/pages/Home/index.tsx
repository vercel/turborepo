import { authRoute } from '@/routes';
import { createRoute } from '@tanstack/react-router';
import { App, Card, Space, Typography } from 'antd';

const { Text } = Typography;

export const coinsRoute = createRoute({
  getParentRoute: () => authRoute,
  path: '/coins',
  component: Home,
});

function Home() {
  const { notification, modal } = App.useApp();

  return (
    <Space>
      <Card title="Coins Generated" style={{ width: 300 }}>
        <div>Home</div>
      </Card>
    </Space>
  );
}

export default Home;

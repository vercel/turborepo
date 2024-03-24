import MyApp from './App';
import { App } from 'antd';
import './index.css';
import 'antd/dist/reset.css';
import ConfigProvider from 'antd/es/config-provider';
import React from 'react';
import ReactDOM from 'react-dom/client';
import { StyleProvider } from '@ant-design/cssinjs';

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <ConfigProvider
      theme={{
        token: {
          colorPrimary: '#9437FD',
          // 148	55	253
          colorFill: '#191939',
          // 25	25	57
        },
      }}
    >
      <StyleProvider hashPriority="high">
        <App>
          <MyApp />
        </App>
      </StyleProvider>
    </ConfigProvider>
  </React.StrictMode>,
);

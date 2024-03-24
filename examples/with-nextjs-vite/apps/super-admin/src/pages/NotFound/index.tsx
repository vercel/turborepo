import { useNavigate } from '@tanstack/react-router';
import Button from 'antd/es/button';
import Result from 'antd/es/result';
import React from 'react';

function NotFound() {
  const navigate = useNavigate({ from: '/404' });
  return (
    <Result
      status="404"
      title="404"
      subTitle="Sorry, the page you visited does not exist."
      extra={
        <Button type="primary" onClick={() => navigate({ to: '/' })}>
          Back Home
        </Button>
      }
    />
  );
}

export default NotFound;

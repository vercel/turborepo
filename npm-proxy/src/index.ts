import { json, raw } from 'body-parser';
import cors from 'cors';
import express, { Application } from 'express';
import expressPino from 'express-pino-logger';
import httpProxy from 'http-proxy';
// @ts-ignore
import modifyResponse from 'node-http-proxy-json';
import pino from 'pino';

const app = express();
const logger = pino();
const reqLogger = expressPino({ logger });
const proxy = httpProxy.createProxyServer({});

interface ServerConfig {
  port: number | string;
}

const notadb = [];
const TOKEN = process.env.GITHUB_TOKEN;
const turboRegistry = 'https://npm.turborepo.com';
const ghRegistry = 'https://npm.pkg.github.com';

proxy.on('proxyRes', function (proxyRes, req, res) {
  if (req.url?.startsWith('/@')) {
    modifyResponse(
      res,
      proxyRes.headers['content-encoding'],
      (body: unknown) => {
        if (body) {
          return JSON.stringify(body, function (key, value) {
            if (value && typeof value === 'object') {
              var replacement: any = {};
              for (var k in value) {
                if (Object.hasOwnProperty.call(value, k)) {
                  replacement[
                    k.replace(/https:\/\/npm.pkg.github.com/g, turboRegistry)
                  ] = value[k];
                }
              }
              return replacement;
            }
            if (value && typeof value === 'string') {
              return value.replace(
                /https:\/\/npm.pkg.github.com/g,
                turboRegistry
              );
            }
            return value;
          });
        }
        return body;
      }
    );
  }
});

export function createServer(config: ServerConfig): Application {
  app.use(reqLogger);
  app.disable('x-powered-by');
  app.use(
    raw({
      type: 'application/octet-stream',
      limit: '100mb',
    })
  );
  app.use(json());
  app.use(cors());
  app.get('/healthz', (req, res) => {
    res.status(200).send('pong');
  });
  app.get('*', (req, res) => {
    const token = req.headers?.authorization?.replace('Bearer ', '');

    logger.info({
      auth: notadb.includes(token ?? ''),
      url: req.url,
    });

    proxy.web(req, res, {
      target: ghRegistry,
      autoRewrite: true,
      changeOrigin: true,
      followRedirects: true,
      preserveHeaderKeyCase: true,
      headers: {
        authorization: 'Bearer ' + TOKEN,
      },
    });
  });
  return app;
}

const server = createServer({ port: process.env.PORT || 5000 });
server.listen(process.env.PORT || 5000, () => {
  logger.info(`Turborepo server running on ${process.env.PORT || 5000}`);
});

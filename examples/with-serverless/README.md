# Turborepo Serverless starter

This is an unofficial Serverless Framework starter Turborepo.

## What's inside?

This turborepo uses [NPM](https://www.npmjs.com/package/download) as a package manager. It includes the following services/apps:

### Services and Packages

- `calculator`: a [TypeScript](https://www.typescriptlang.org/) sample private module
- `simple-calculator-api`: a simple [Serverless Framework](https://serverless.com/) Calculator REST API
- `scripts`: Jest configurations
- `logger`: Isomorphic logger (a small wrapper around @aws-lambda-powertools/logger)
- `tsconfig`: tsconfig.json;s used throughout the monorepo

Each package/app is 100% [TypeScript](https://www.typescriptlang.org/).

### Serverless Framework with AWS Lambda

This repo is configured to be built with AWS Lambda, and AWS API Gateway. To build all apps in this repo:

```
# Create an AWS IAM credential
# Refer: https://aws.amazon.com/premiumsupport/knowledge-center/create-access-key/
export AWS_ACCESS_KEY_ID=<your-aws-access-key-id>
export AWS_SECRET_ACCESS_KEY=<your-aws-secret-access-key>
export AWS_DEFAULT_REGION=us-west-1

# Install dependencies
npm install

# Build packages
npm run build:packages

# Build service environment using turbo script
# For local:
npm run sls:package:local
# For development:
npm run sls:package:dev
# For production:
npm run sls:package:prod

# Deploy the sevices
# For development:
npm run sls:deploy:local
# For development:
npm run sls:deploy:dev
# For production:
npm run sls:deploy:prod
```

Access the REST API via the AWS API Gateway endpoint generated.

To cleanup the resources:

```
# This will delete the stack from the environment. 
npm run sls:remove:local
npm run sls:remove:dev
npm run sls:remove:prod
```

### Utilities

This Turborepo has some additional tools already setup for you:

- [TypeScript](https://www.typescriptlang.org/) for static type checking
- [ESLint](https://eslint.org/) for code linting
- [Jest](https://jestjs.io) test runner for all things JavaScript
- [Prettier](https://prettier.io) for code formatting

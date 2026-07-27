// Copyright (c) 2026 StellarDevTools
// SPDX-License-Identifier: MIT

import swaggerJsdoc from 'swagger-jsdoc';
import swaggerUi from 'swagger-ui-express';
import { versions } from '../config/versions.js';

const versionedRoutePrefixes = [
  '/compile',
  '/deploy',
  '/invoke',
  '/identity',
  '/lottery',
];

const options = {
  definition: {
    openapi: '3.0.0',
    info: {
      title: 'Soroban Playground API',
      version: '1.0.0',
      description:
        'REST API for compiling, deploying, and invoking Soroban smart contracts on Stellar.',
    },
    servers: [
      { url: '/api', description: 'Default server' },
      ...Object.keys(versions).map((version) => ({
        url: `/api/${version}`,
        description: `${version.toUpperCase()} API server`,
      })),
    ],
    tags: [
      { name: 'Versioning', description: 'API version discovery and routing' },
      { name: 'Contract Compiler', description: 'Synchronous and Asynchronous WASM Compilation' },
      { name: 'Deploy & Invoke', description: 'Contract deployment and invocation operations' },
      { name: 'RPC Network Manager', description: 'Circuit breaker & RPC health status' },
      ...Object.keys(versions).map((version) => ({
        name: `API ${version}`,
        description: `${version.toUpperCase()} endpoints`,
      })),
    ],
    components: {
      securitySchemes: {
        bearerAuth: {
          type: 'http',
          scheme: 'bearer',
          bearerFormat: 'JWT',
        },
      },
      schemas: {
        Error: {
          type: 'object',
          properties: {
            error: { type: 'string' },
            message: { type: 'string' },
          },
        },
        CompileRequest: {
          type: 'object',
          required: ['source'],
          properties: {
            source: { type: 'string', description: 'Rust source code for Soroban smart contract' },
            contractName: { type: 'string', description: 'Optional name of the contract' },
          },
        },
        AsyncCompileResponse: {
          type: 'object',
          properties: {
            jobId: { type: 'string', description: 'Unique background compilation job ID' },
            status: { type: 'string', example: 'queued' },
            createdAt: { type: 'string', format: 'date-time' },
          },
        },
        CompileJobStatus: {
          type: 'object',
          properties: {
            jobId: { type: 'string' },
            status: { type: 'string', enum: ['queued', 'processing', 'completed', 'failed'] },
            result: { type: 'object' },
            error: { type: 'string' },
          },
        },
        RpcStatusResponse: {
          type: 'object',
          properties: {
            activeEndpoint: { type: 'string' },
            circuitBreakerState: { type: 'string', enum: ['CLOSED', 'OPEN', 'HALF_OPEN'] },
            endpoints: {
              type: 'array',
              items: {
                type: 'object',
                properties: {
                  url: { type: 'string' },
                  isHealthy: { type: 'boolean' },
                  failCount: { type: 'integer' },
                },
              },
            },
          },
        },
      },
    },
  },
  apis: ['./src/routes/**/*.js', './src/docs/*.doc.js'],
};

function cloneOperation(operation, version) {
  const cloned = JSON.parse(JSON.stringify(operation));
  const tags = new Set([`API ${version}`, ...(cloned.tags || [])]);
  cloned.tags = Array.from(tags);
  return cloned;
}

function clonePathItem(pathItem, version) {
  const cloned = JSON.parse(JSON.stringify(pathItem));
  for (const [method, operation] of Object.entries(cloned)) {
    if (operation && typeof operation === 'object') {
      cloned[method] = cloneOperation(operation, version);
    }
  }
  return cloned;
}

function isVersionablePath(pathName) {
  if (!pathName.startsWith('/api/')) return false;
  const pathWithoutApiPrefix = pathName.slice('/api'.length);
  return versionedRoutePrefixes.some(
    (prefix) =>
      pathWithoutApiPrefix === prefix ||
      pathWithoutApiPrefix.startsWith(`${prefix}/`)
  );
}

function withVersionedDocumentation(spec) {
  const documentedSpec = {
    ...spec,
    paths: { ...(spec.paths || {}) },
  };

  for (const [pathName, pathItem] of Object.entries(spec.paths || {})) {
    const versionMatch = pathName.match(/^\/api\/(v\d+)(?:\/|$)/);
    if (versionMatch) {
      documentedSpec.paths[pathName] = clonePathItem(pathItem, versionMatch[1]);
      continue;
    }

    if (!isVersionablePath(pathName)) continue;

    const legacyVersion = 'v1';
    documentedSpec.paths[pathName] = clonePathItem(pathItem, legacyVersion);

    for (const version of Object.keys(versions)) {
      const versionedPath = pathName.replace('/api', `/api/${version}`);
      if (!documentedSpec.paths[versionedPath]) {
        documentedSpec.paths[versionedPath] = clonePathItem(pathItem, version);
      }
    }
  }

  return documentedSpec;
}

export const swaggerSpec = withVersionedDocumentation(swaggerJsdoc(options));

export function setupSwagger(app) {
  app.get('/api-docs/spec.json', (_req, res) => {
    res.setHeader('Content-Type', 'application/json');
    res.send(swaggerSpec);
  });

  app.use(
    '/api-docs',
    swaggerUi.serve,
    swaggerUi.setup(swaggerSpec, {
      customSiteTitle: 'Soroban Playground API Docs',
      swaggerOptions: {
        persistAuthorization: true,
        displayRequestDuration: true,
        filter: true,
        tryItOutEnabled: true,
      },
    })
  );
}

// Copyright (c) 2026 StellarDevTools
// SPDX-License-Identifier: MIT

import config from '../config/index.js';

const DEFAULT_FALLBACK_ENDPOINTS = [
  process.env.SOROBAN_RPC_URL || config?.soroban?.rpcUrl || 'https://soroban-testnet.stellar.org',
  'https://rpc-futurenet.stellar.org',
  'https://stellar-community.org/rpc',
  'http://localhost:8000/soroban/rpc',
];

export const CIRCUIT_STATES = {
  CLOSED: 'CLOSED',
  OPEN: 'OPEN',
  HALF_OPEN: 'HALF_OPEN',
};

class SorobanRpcManager {
  constructor() {
    const rawFallbacks = process.env.SOROBAN_RPC_FALLBACK_URLS
      ? process.env.SOROBAN_RPC_FALLBACK_URLS.split(',').map((u) => u.trim())
      : DEFAULT_FALLBACK_ENDPOINTS;

    // Deduplicate endpoints
    this.endpoints = Array.from(new Set(rawFallbacks)).map((url) => ({
      url,
      state: CIRCUIT_STATES.CLOSED,
      failCount: 0,
      lastFailureTime: null,
      isHealthy: true,
    }));

    this.failureThreshold = Number.parseInt(process.env.RPC_FAILURE_THRESHOLD || '3', 10);
    this.resetTimeoutMs = Number.parseInt(process.env.RPC_RESET_TIMEOUT_MS || '30000', 10);
    this.activeEndpointIndex = 0;
  }

  get activeEndpoint() {
    return this.endpoints[this.activeEndpointIndex] || this.endpoints[0];
  }

  checkCircuitStates() {
    const now = Date.now();
    for (const ep of this.endpoints) {
      if (
        ep.state === CIRCUIT_STATES.OPEN &&
        ep.lastFailureTime &&
        now - ep.lastFailureTime > this.resetTimeoutMs
      ) {
        ep.state = CIRCUIT_STATES.HALF_OPEN;
      }
    }
  }

  async executeRpcCall(callFn) {
    this.checkCircuitStates();

    let lastError = null;
    const startIndex = this.activeEndpointIndex;

    for (let i = 0; i < this.endpoints.length; i++) {
      const idx = (startIndex + i) % this.endpoints.length;
      const ep = this.endpoints[idx];

      if (ep.state === CIRCUIT_STATES.OPEN) {
        continue;
      }

      try {
        const result = await callFn(ep.url);

        // Success: reset failures and set state to CLOSED
        ep.failCount = 0;
        ep.state = CIRCUIT_STATES.CLOSED;
        ep.isHealthy = true;
        this.activeEndpointIndex = idx;

        return result;
      } catch (err) {
        lastError = err;
        ep.failCount += 1;
        ep.lastFailureTime = Date.now();

        if (ep.failCount >= this.failureThreshold || ep.state === CIRCUIT_STATES.HALF_OPEN) {
          ep.state = CIRCUIT_STATES.OPEN;
          ep.isHealthy = false;
          console.warn(
            `[RPC Circuit Breaker] Tripped OPEN for endpoint ${ep.url} (failures: ${ep.failCount})`
          );
        }
      }
    }

    throw new Error(
      `All Soroban RPC endpoints failed or are circuit breaker OPEN. Last error: ${
        lastError?.message || 'Unknown error'
      }`
    );
  }

  getStatus() {
    this.checkCircuitStates();
    return {
      activeEndpoint: this.activeEndpoint.url,
      circuitBreakerState: this.activeEndpoint.state,
      endpoints: this.endpoints.map((ep) => ({
        url: ep.url,
        state: ep.state,
        isHealthy: ep.isHealthy,
        failCount: ep.failCount,
        lastFailureTime: ep.lastFailureTime
          ? new Date(ep.lastFailureTime).toISOString()
          : null,
      })),
    };
  }

  reset() {
    for (const ep of this.endpoints) {
      ep.state = CIRCUIT_STATES.CLOSED;
      ep.failCount = 0;
      ep.lastFailureTime = null;
      ep.isHealthy = true;
    }
    this.activeEndpointIndex = 0;
  }
}

export const sorobanRpcManager = new SorobanRpcManager();
export default sorobanRpcManager;

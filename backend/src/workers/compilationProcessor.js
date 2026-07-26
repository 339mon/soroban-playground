// Copyright (c) 2026 StellarDevTools
// SPDX-License-Identifier: MIT

import { compileContract } from '../services/compileService.js';
import { getWss } from '../websocket.js';

/**
 * Sandboxed processor for asynchronous WASM compilation.
 * Runs in background worker pool.
 */
export default async function compilationProcessor(job) {
  console.log(
    `[Compilation Worker] Processing job ${job.id} (Attempt ${job.attemptsMade + 1})`
  );

  const { source, contractName = 'soroban_contract' } = job.data;

  if (!source) {
    throw new Error('Compilation job missing required "source" payload.');
  }

  try {
    const result = await compileContract({ source, contractName });

    // Emit WebSocket completion notification if WS server is running
    try {
      const wss = getWss();
      if (wss && wss.clients) {
        const payload = JSON.stringify({
          type: 'compilation:completed',
          jobId: job.id,
          status: 'completed',
          wasmUrl: result.wasmUrl,
          hash: result.hash,
        });
        wss.clients.forEach((client) => {
          if (client.readyState === 1) {
            client.send(payload);
          }
        });
      }
    } catch {
      // WS notification is best-effort
    }

    console.log(`[Compilation Worker] Job ${job.id} compiled successfully.`);
    return {
      success: true,
      jobId: job.id,
      hash: result.hash,
      wasmUrl: result.wasmUrl,
      sizeBytes: result.sizeBytes,
      durationMs: result.durationMs,
    };
  } catch (err) {
    console.error(`[Compilation Worker] Job ${job.id} failed:`, err.message);

    // Emit WebSocket failure notification
    try {
      const wss = getWss();
      if (wss && wss.clients) {
        const payload = JSON.stringify({
          type: 'compilation:failed',
          jobId: job.id,
          status: 'failed',
          error: err.message,
        });
        wss.clients.forEach((client) => {
          if (client.readyState === 1) {
            client.send(payload);
          }
        });
      }
    } catch {
      // WS notification is best-effort
    }

    throw err;
  }
}

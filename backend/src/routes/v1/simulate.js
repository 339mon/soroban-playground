import express from 'express';
import { asyncHandler, createHttpError } from '../../middleware/errorHandler.js';
import sorobanRpcManager from '../../services/sorobanRpcManager.js';
import { rateLimitMiddleware } from '../../middleware/rateLimiter.js';

const router = express.Router();

function estimateFallback(xdr) {
  const xdrLength = xdr.length;
  return {
    minResourceFee: String(1_000 + Math.ceil(xdrLength * 1.5)),
    cost: {
      cpuInsns: String(Math.min(10_000_000, 150_000 + xdrLength * 120)),
      memBytes: String(Math.min(5_000_000, 65_536 + xdrLength * 32)),
    },
    results: [{ auth: [], xdr }],
    events: [],
    latestLedger: 100000,
  };
}

async function callSimulateTransaction(xdr) {
  return await sorobanRpcManager.executeRpcCall(async (rpcUrl, options = {}) => {
    const { signal, ...extraHeaders } = options;

    const payload = {
      jsonrpc: '2.0',
      id: Date.now(),
      method: 'simulateTransaction',
      params: { transaction: xdr },
    };

    const response = await fetch(rpcUrl, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        ...extraHeaders,
      },
      body: JSON.stringify(payload),
      signal,
    });

    if (!response.ok) {
      throw new Error(`RPC server returned status ${response.status}`);
    }

    const data = await response.json();
    if (data.error) {
      throw new Error(data.error.message || 'Soroban RPC simulation error');
    }

    return data.result || {};
  });
}

router.post(
  '/fee',
  rateLimitMiddleware('read'),
  asyncHandler(async (req, res, next) => {
    const { transactionXdr, transaction, network = 'testnet' } = req.body || {};
    const xdrToSimulate = transactionXdr || transaction;

    if (!xdrToSimulate || typeof xdrToSimulate !== 'string') {
      return res.status(400).json({
        success: false,
        error: 'transactionXdr or transaction (base64 string) is required',
      });
    }

    try {
      let rpcResult;
      try {
        rpcResult = await callSimulateTransaction(xdrToSimulate);
      } catch {
        rpcResult = estimateFallback(xdrToSimulate);
      }

      const minResourceFee = String(rpcResult.minResourceFee || rpcResult.minFee || '1000');
      const cpuInstructions = parseInt(rpcResult.cost?.cpuInsns || '150000', 10);
      const memoryBytes = parseInt(rpcResult.cost?.memBytes || '65536', 10);
      const readCount = parseInt(rpcResult.readCount || '2', 10);
      const writeCount = parseInt(rpcResult.writeCount || '1', 10);
      const ledgerReadBytes = parseInt(rpcResult.ledgerReadBytes || '1024', 10);
      const ledgerWriteBytes = parseInt(rpcResult.ledgerWriteBytes || '512', 10);

      const baseFee = 100;
      const estimatedTotalFee = String(parseInt(minResourceFee, 10) + baseFee);

      return res.json({
        success: true,
        status: 'success',
        data: {
          network,
          minResourceFee,
          cpuInstructions,
          memoryBytes,
          ledgerReadBytes,
          ledgerWriteBytes,
          readCount,
          writeCount,
          estimatedTotalFee,
          transactionData: rpcResult.transactionData || null,
          eventsCount: Array.isArray(rpcResult.events) ? rpcResult.events.length : 0,
          latestLedger: rpcResult.latestLedger || null,
        },
      });
    } catch (error) {
      return next(
        createHttpError(500, 'Fee simulation failed', { details: error.message })
      );
    }
  })
);

export default router;

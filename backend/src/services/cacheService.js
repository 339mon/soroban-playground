// Copyright (c) 2026 StellarDevTools
// SPDX-License-Identifier: MIT

import Redis from 'ioredis';

const DEFAULT_TTL_SECONDS = 300; // 5 minutes
const POPULARITY_TTL_SECONDS = 86400 * 7; // 7 days
const MAX_SMART_TTL_SECONDS = 1800; // 30 minutes
const BASE_SMART_TTL_SECONDS = 300; // 5 minutes
const SMART_TTL_POPULARITY_STEP_SECONDS = 60;

class CacheService {
  constructor() {
    this.redis = null;
    this.isConnected = false;
  }

  async initialize() {
    try {
      this.redis = new Redis({
        host: process.env.REDIS_HOST || 'localhost',
        port: process.env.REDIS_PORT || 6379,
        password: process.env.REDIS_PASSWORD || undefined,
        db: process.env.REDIS_DB || 0,
        retryDelayOnFailover: 100,
        maxRetriesPerRequest: 3,
        lazyConnect: true,
      });

      this.redis.on('connect', () => {
        this.isConnected = true;
      });

      this.redis.on('error', (err) => {
        console.error('Redis connection error:', err);
        this.isConnected = false;
      });

      this.redis.on('close', () => {
        this.isConnected = false;
      });

      await this.redis.connect();
      return true;
    } catch (error) {
      console.error('Redis initialization failed:', error);
      this.isConnected = false;
      return false;
    }
  }

  /**
   * Delete keys matching a prefix using SCAN (non-blocking).
   * Avoids the O(N) KEYS command that blocks the Redis event loop.
   */
  async #deleteByPattern(pattern) {
    if (!this.isConnected || !this.redis) return 0;

    let cursor = '0';
    let deleted = 0;
    do {
      const [next, keys] = await this.redis.scan(
        cursor,
        'MATCH',
        pattern,
        'COUNT',
        100
      );
      cursor = next;
      if (keys.length > 0) {
        await this.redis.del(...keys);
        deleted += keys.length;
      }
    } while (cursor !== '0');
    return deleted;
  }

  /**
   * Collect key-value pairs matching a prefix using SCAN.
   * Returns an array of { key, value } objects.
   */
  async #scanPattern(pattern) {
    const results = [];
    let cursor = '0';
    do {
      const [next, keys] = await this.redis.scan(
        cursor,
        'MATCH',
        pattern,
        'COUNT',
        100
      );
      cursor = next;
      if (keys.length > 0) {
        const pipeline = this.redis.pipeline();
        keys.forEach((key) => pipeline.get(key));
        const values = await pipeline.exec();
        values.forEach(([err, val], idx) => {
          if (!err && val) {
            results.push({ key: keys[idx], value: val });
          }
        });
      }
    } while (cursor !== '0');
    return results;
  }

  generateSearchKey(query, filters, pagination) {
    const keyData = { query, filters, pagination };
    return `search:${Buffer.from(JSON.stringify(keyData)).toString('base64')}`;
  }

  generateFacetKey(query) {
    return `facets:${Buffer.from(query).toString('base64')}`;
  }

  generateAutocompleteKey(query) {
    return `autocomplete:${Buffer.from(query).toString('base64')}`;
  }

  async get(key) {
    if (!this.isConnected) return null;

    try {
      const cached = await this.redis.get(key);
      return cached ? JSON.parse(cached) : null;
    } catch (error) {
      console.error('Cache get error:', error);
      return null;
    }
  }

  async set(key, data, ttl = DEFAULT_TTL_SECONDS) {
    if (!this.isConnected) return false;

    try {
      await this.redis.setex(key, ttl, JSON.stringify(data));
      return true;
    } catch (error) {
      console.error('Cache set error:', error);
      return false;
    }
  }

  async del(key) {
    if (!this.isConnected) return false;

    try {
      await this.redis.del(key);
      return true;
    } catch (error) {
      console.error('Cache delete error:', error);
      return false;
    }
  }

  async has(key) {
    if (!this.isConnected) return false;

    try {
      const exists = await this.redis.exists(key);
      return exists === 1;
    } catch (error) {
      console.error('Cache exists error:', error);
      return false;
    }
  }

  async clearSearchCache() {
    if (!this.isConnected) return false;

    try {
      await this.#deleteByPattern('search:*');
      return true;
    } catch (error) {
      console.error('Cache clear error:', error);
      return false;
    }
  }

  async incrementSearchPopularity(query) {
    if (!this.isConnected) return false;

    try {
      const key = `popular:${query}`;
      await this.redis.incr(key);
      await this.redis.expire(key, POPULARITY_TTL_SECONDS);
      return true;
    } catch (error) {
      console.error('Popularity increment error:', error);
      return false;
    }
  }

  async getPopularSearches(limit = 10) {
    if (!this.isConnected) return [];

    try {
      const entries = await this.#scanPattern('popular:*');
      const searches = entries.map(({ key, value }) => ({
        query: key.replace('popular:', ''),
        count: parseInt(value, 10),
      }));

      return searches.sort((a, b) => b.count - a.count).slice(0, limit);
    } catch (error) {
      console.error('Popular searches cache error:', error);
      return [];
    }
  }

  async cacheSearchResults(query, filters, pagination, results) {
    if (!this.isConnected) return false;

    try {
      const key = this.generateSearchKey(query, filters, pagination);

      const popularityScore = await this.getQueryPopularity(query);
      const ttl = Math.min(
        BASE_SMART_TTL_SECONDS +
          popularityScore * SMART_TTL_POPULARITY_STEP_SECONDS,
        MAX_SMART_TTL_SECONDS
      );

      await this.set(key, results, ttl);
      await this.incrementSearchPopularity(query);

      return true;
    } catch (error) {
      console.error('Search results caching error:', error);
      return false;
    }
  }

  async getQueryPopularity(query) {
    if (!this.isConnected) return 0;

    try {
      const key = `popular:${query}`;
      const count = await this.redis.get(key);
      return count ? parseInt(count, 10) : 0;
    } catch (error) {
      console.error('Query popularity error:', error);
      return 0;
    }
  }

  async healthCheck() {
    if (!this.isConnected) {
      return { status: 'disconnected', message: 'Redis not connected' };
    }

    try {
      const pong = await this.redis.ping();
      const info = await this.redis.info('memory');

      return {
        status: 'connected',
        message: 'Redis is healthy',
        ping: pong,
        memory: info,
      };
    } catch (error) {
      return {
        status: 'error',
        message: error.message,
      };
    }
  }

  async getCacheAdminSnapshot() {
    return {
      cacheVersion: 'v1',
      memoryEntries: this.isConnected ? await this.redis.dbsize() : 0,
      isConnected: this.isConnected,
    };
  }

  async warmCache({ hashes, top }) {
    return { warmed: hashes || [], warmedCount: (hashes || []).length };
  }

  async invalidateCache({ hash, dependency, namespace }) {
    if (hash) {
      await this.del(hash);
    }
    return { success: true };
  }

  async bumpCacheVersion({ version }) {
    return version || 'v2';
  }

  async close() {
    if (this.redis) {
      await this.redis.quit();
      this.isConnected = false;
    }
  }
}

export default new CacheService();

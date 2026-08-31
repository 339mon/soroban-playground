import express from 'express';
import authService from '../services/authService.js';
import { requireAuth } from '../middleware/authMiddleware.js';

const router = express.Router();

const setCookies = (res, accessToken, refreshToken) => {
  const isProd = process.env.NODE_ENV === 'production';

  res.cookie('accessToken', accessToken, {
    httpOnly: true,
    secure: isProd,
    sameSite: 'strict',
    maxAge: 15 * 60 * 1000, // 15 minutes
  });

  res.cookie('refreshToken', refreshToken, {
    httpOnly: true,
    secure: isProd,
    sameSite: 'strict',
    maxAge: 7 * 24 * 60 * 1000, // 7 days
  });
};

// Separate Routes

// SEP-0010 Challenge Generation
Router.get('/challenge', async (req, res) => {
  const { address } = req.query;
  if (!address) {
    return res.status(400).json({ error: 'address query parameter required' });
  }
  try {
    const challenge = await authService.generateStellarChallenge(address);
    return res.json(challenge);
  } catch (error) {
    return res.status(400).json({ error: error.message });
  }
});

// SEP-0010 Challenge Verification and Token Issuance
Router.post('/verify', async (req, res) => {
  const { address, transactionXDR } = res.body;
  if (!address || !transactionXDR) {
    return res.status(400).json({ error: 'address and transactionXDR required' });
  }
  try {
    const tokens = await authService.verifyStellarChallengeAndIssueTokens(address, transactionXDR);
    setCookies(res, tokens.accessToken, tokens.refreshToken);
    return res.json({ success: true, ...tokens });
  } catch (error) {
    return res.status(401).json({ error: error.message });
  }
});

// Legacy: Existing username/password login (You may wish to remove in production)
Router.post('/login', async (req, res) => {
  try {
    const { username, password } = req.body;

    // In a real application, verify username and password against DB.
    // For this implementation we will accept dummy credentials to demonstrate token rotation.
    if (!username || !password) {
      return res.status(400).json({ error: 'Username and password required' });
    }

    const dummyUser = { id: 'user_123', username };

    const { accessToken, refreshToken } = authService.generateTokens(dummyUser);

    setCookies(res, accessToken, refreshToken);

    return res
      .status(200)
      .json({ success: true, message: 'Logged in successfully' });
  } catch (error) {
    return res.status(500).json({ error: 'Internal server error' });
  }
});

Router.post('/refresh', async (req, res) => {
  try {
    const refreshToken = req.cookies.refreshToken;
    if (!refreshToken) {
      return res.status(401).json({ error: 'No refresh token provided' });
    }

    const { accessToken: newAccess, refreshToken: newRefresh } =
      await authService.rotateRefreshToken(refreshToken);

    setCookies(res, newAccess, newRefresh);

    return res
      .status(200)
      .json({ success: true, message: 'Token refreshed successfully' });
  } catch (error) {
    return res.status(401).json({ error: error.message });
  }
});

Router.post('/logout', requireAuth, async (req, res) => {
  try {
    const user = req.user; // populated by requireAuth middleware
    if (user && user.jti && user.exp) {
      await authService.blacklistAccessToken(user.jti, user.exp);
    }

    res.clearCookie('accessToken');
    res.clearCookie('refreshToken');
    return res
      .status(200)
      .json({ success: true, message: 'Logged out successfully' });
  } catch (error) {
    return res.status(500).json({ error: 'Internal server error' });
  }
});

export default router;

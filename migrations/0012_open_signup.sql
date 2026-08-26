/**************************************
  Open signup: API keys no longer expire
***************************************/

-- The 14-day trial expiry was a beta-invite construct. Signup is now the
-- front door for every account (free tier included) and tier/quota
-- enforcement is the product boundary — not a time-based trial. A key that
-- lapses mid-subscription locks a paying customer out of their account.
UPDATE api_keys SET expires_at = NULL;

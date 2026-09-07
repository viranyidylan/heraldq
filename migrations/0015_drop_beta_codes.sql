/**************************************
  Drop the beta invite-code machinery
***************************************/

-- Signup went open in 0012; nothing has issued or checked an invite code
-- since. The table, its FK to tenants, and its redemption CHECK were the
-- last remnants of the beta-invite flow.
--
-- This is a one-way door: redemption history is discarded. It is history of
-- a closed beta, not of anything a customer or the billing mirror reads.
DROP TABLE IF EXISTS beta_codes;

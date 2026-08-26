/**************************************
  Normalize stored emails to lowercase
***************************************/

-- The Paddle<->tenant linkage matches on email, and emails are now written
-- lowercased on both sides (signup and the customer mirror). Lowercase the
-- existing rows so old accounts match too.
--
-- Note: if a beta-era database somehow holds both "A@x.com" and "a@x.com",
-- this UPDATE fails on the unique index — resolve the duplicate manually
-- and re-run. One account per person makes that a non-issue in practice.
UPDATE tenants SET email = lower(email) WHERE email <> lower(email);
UPDATE paddle_customers SET email = lower(email) WHERE email <> lower(email);

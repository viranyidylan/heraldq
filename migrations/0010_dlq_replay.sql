/**************************************
  Self-serve DLQ replay
***************************************/

-- A replayed DLQ row stays visible (it is still the record of the original
-- failure) but is stamped with when it was replayed and the id of the new
-- delivery attempt, so the tenant can follow the trail and the API can
-- refuse accidental double-replays.
ALTER TABLE dead_letter_queue
    ADD COLUMN IF NOT EXISTS replayed_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS replayed_as TEXT;

-- Record how many delivery attempts a webhook took before reaching a terminal
-- state. The graphile_worker job row (which carries `attempts`) is deleted once
-- a job completes, so without this the retry count is lost the moment a webhook
-- succeeds or is dead-lettered. Nullable: existing rows predate the column.
ALTER TABLE webhook_deliveries ADD COLUMN attempts SMALLINT;
ALTER TABLE dead_letter_queue ADD COLUMN attempts SMALLINT;

INSERT INTO beta_codes (code)
SELECT 'BETA_' || upper(substring(md5(random()::text), 1, 8))
FROM generate_series(1, 20)
RETURNING code;

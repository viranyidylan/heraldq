INSERT INTO beta_codes (code)                                   
SELECT 'BETA_' || upper(substring(md5(random()::text), 1, 8))   -- concat 'BETA_'+ (8 random chars eg. '9DM27VG8')
FROM generate_series(1, 20)                                     -- create 20 of them
RETURNING code;

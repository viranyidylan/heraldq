#!/bin/bash
set -e

TOKEN=$(openssl rand -hex 32)
HASH=$(echo -n "$TOKEN" | sha256sum | awk '{print $1}')

TENANT_ID=$(psql -U postgres -d sliq -t -A -c \
  "INSERT INTO tenants (name) VALUES ('$1') RETURNING id;" | head -1)

psql -U postgres -d sliq -c \
  "INSERT INTO api_keys (tenant_id, key_hash, name) VALUES ('$TENANT_ID', '$HASH', '$1 key');"

echo "Tenant: $1 ($TENANT_ID)"
echo "API Key: $TOKEN"

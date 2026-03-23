#!/bin/bash
set -e

# create token
TOKEN=$(openssl rand -hex 32)
# hash it
HASH=$(echo -n "$TOKEN" | sha256sum | awk '{print $1}')

# create a tenant and return id
TENANT_ID=$(psql -U postgres -d sliq -t -A -c \
  "INSERT INTO tenants (name) VALUES ('$1') RETURNING id;" | head -1)

# insert the api key
psql -U postgres -d sliq -c \
  "INSERT INTO api_keys (tenant_id, key_hash, name) VALUES ('$TENANT_ID', '$HASH', '$1 key');"

echo "Tenant: $1 ($TENANT_ID)"
echo "API Key: $TOKEN"

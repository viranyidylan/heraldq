#!/bin/bash
set -e

psql -U postgres <<EOF
CREATE USER postgres WITH PASSWORD 'xi23pe';
CREATE DATABASE sliq OWNER postgres;
EOF

echo "Done. Database 'sliq' created with user 'postgres'."

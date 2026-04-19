  
  # curl -v http://localhost:3000/webhook \
  #   -X POST \
  #   -H "Content-Type: application/json" \
  #   -H "Authorization: Bearer 9b4f9a0e76b070c800756831924299a7d8372365eda134ebc9e94787306c14c4" \
  #   -d '{"destination": "https://webhook.site/92d79c0e-c937-45d0-8513-e8e4d4b1a925", 
  #         "payload": {
  #             "event": "order.created",
  #             "order_id": "ord_123abc",
  #             "customer_id": "cust_456def",
  #             "amount": 12999,
  #             "currency": "USD",
  #             "items": [{"sku": "WIDGET-001", "qty": 2}],
  #             "timestamp": "2026-04-07T14:30:22Z"
  #           }
  #       }'

    # -H "Authorization: Bearer sliq_a070c4c332deed9d302b2257fb146754" \ different tenant

# Get stats
  # curl http://localhost:3000/summary \
  #   -H "Authorization: Bearer sliq_a070c4c332deed9d302b2257fb146754" \

# Get dlq 
  # curl -v http://localhost:3000/dlq \
  #   -H "Authorization: Bearer 9b4f9a0e76b070c800756831924299a7d8372365eda134ebc9e94787306c14c4" \

curl -v https://sliq-graphile-api-production.up.railway.app/webhook \
   -X POST \
   -H "Content-Type: application/json" \
   -H "Authorization: Bearer sliq_6244b5d3f0a0efbcea9b1b302df975e9" \
   -d '{"destination": "https://webhook.site/92d79c0e-c937-45d0-8513-e8e4d4b1a925",
         "payload": {
              "event": "order.created",
              "order_id": "ord_123abc",
              "customer_id": "cust_456def",
              "amount": 12999,
              "currency": "USD",
              "items": [{"sku": "WIDGET-001", "qty": 2}],
              "timestamp": "2026-04-07T14:30:22Z"
            }
        }'


#  Prod test key for tenant 'My Company'
#  -H "Authorization: Bearer sliq_6244b5d3f0a0efbcea9b1b302df975e9" \ //prod test account 

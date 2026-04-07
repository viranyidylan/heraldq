  curl -v http://localhost:3000/webhook \
    -X POST \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer 9b4f9a0e76b070c800756831924299a7d8372365eda134ebc9e94787306c14c4" \
    -d '{"destination": "http://localhost:3000/mock", 
          "payload": {"hello":"world"}}'

    # -d '{
    #   "destination": "https://webhook.site/a2b26216-4f6f-4e67-bf60-237c54da4555", 
    #   "payload": {
    #     "hello": "all"
    #     }
    #   }'

# curl -v https://sliq-graphile-api-production.up.railway.app/webhook \
#    -X POST \
#    -H "Content-Type: application/json" \
#    -H "Authorization: Bearer sliq_6244b5d3f0a0efbcea9b1b302df975e9" \
#    -d '{"destination": "https://webhook.site/a2b26216-4f6f-4e67-bf60-237c54da4555",
#          "payload": {
#               "event": "order.created",
#               "order_id": "ord_123abc",
#               "customer_id": "cust_456def",
#               "amount": 12999,
#               "currency": "USD",
#               "items": [{"sku": "WIDGET-001", "qty": 2}],
#               "timestamp": "2026-04-07T14:30:22Z"
#             }
#         }'
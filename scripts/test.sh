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
  curl -v http://localhost:3000/v1/webhook \
    -X POST \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer sliq_2c0451ae9deac74c5043c75ee3aecddf" \
    -d '{"destination": "https://localhost:3000/v1/mock",
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

  #"https://169.254.169.254/latest/meta-data/",

  # curl -v https://sliq-graphile-api-production.up.railway.app/v1/webhook \
  #  -H "Content-Type: application/json" \
  #  -H "Authorization: Bearer sliq_1f55c0a37aa544a855d59b103fd9d34f" \
  #  -d '{"destination": "https://webhook.site/19b83c6a-6f50-484b-8b03-5f5ee0f44d4a",
  #        "payload": {
  #             "event": "order.created",
  #             "order_id": "ord_123abc",
  #             "customer_id": "cust_456def",
  #             "amount": 12999,
  #             "currency": "USD",
  #             "items": [{"sku": "WIDGET-001", "qty": 2}],
  #             "timestamp": "2026-04-07T14:30:22Z"
  #           }
  #       }'
  
  # curl -v http://localhost:3000/v1/webhook \
  #   -X POST \
  #   -H "Content-Type: application/json" \
  #   -H "Authorization: Bearer sliq_2c0451ae9deac74c5043c75ee3aecddf" \
  #     -d '{"destination": "https://httpbin.org/status/404",
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


      # curl -v http://localhost:3000/v1/webhook \
      #   -X POST \
      #   -H 'content-type: application/json' \
      #   -H 'authorization: Bearer sliq_2c0451ae9deac74c5043c75ee3aecddf' \
      #   -d '{"destination":"https://webhook.site/3051b458-4d9d-48c6-be05-980a74d3dce1", "payload":{"hello":"world"}}'


    # -d '{"destination": "https://webhook.site/3051b458-4d9d-48c6-be05-980a74d3dce1", 
    # -H "Authorization: Bearer sliq_a070c4c332deed9d302b2257fb146754" \ different tenant

# Get stats
  # curl http://localhost:3000/v1/summary \
  #   -H "Authorization: Bearer sliq_a070c4c332deed9d302b2257fb146754" \

# Get dlq 
  # curl -v http://localhost:3000/v1/dlq \
  #   -H "Authorization: Bearer 9b4f9a0e76b070c800756831924299a7d8372365eda134ebc9e94787306c14c4" \

# curl -v https://sliq-graphile-api-production.up.railway.app/v1/webhook \
#    -X POST \
#    -H "Content-Type: application/json" \
#    -H "Authorization: Bearer sliq_6244b5d3f0a0efbcea9b1b302df975e9" \
#    -d '{"destination": "https://webhook.site/92d79c0e-c937-45d0-8513-e8e4d4b1a925",
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

#  Prod test key for tenant 'My Company'
#  -H "Authorization: Bearer sliq_6244b5d3f0a0efbcea9b1b302df975e9" \ //prod test account 

# /signup test
# curl -i -X POST http://localhost:3000/v1/signup \
#   -H "Content-Type: application/json" \
#   -d '{ "name": "DylanCo", "email": "dylan.viranyi@gmail.com", "invite_code": "BETA_MWkkEeBv" }'


# Comment goes here
# PROD CODES
# // email: 
# {
#     "tenant_id":"62dbcbcb-7174-42fc-a347-7ffb13af1a51",
#     "api_key":"sliq_1f55c0a37aa544a855d59b103fd9d34f",
#     "hmac_secret":"318e84bde966d1498830dabdbe9091f1b250707c8fdfda92ca42c45dd899e2ec91f0c08e21e6e47362367d49ac34ca78668324d24bac0e0aa63f25174534c85b",
#     "expires_at":"2026-06-18T06:53:05.462929416Z"
# }

# idk what this one is
#{
# "tenant_id":"65403713-d01a-4bdd-ad03-8b5a6a9a3263",
# "api_key":"sliq_2c0451ae9deac74c5043c75ee3aecddf",
# "hmac_secret":"ade549b2853c7c3f52f426979d935b02a5ee4ecb10f9307e8a931800ebb4b51cddca6a303a28375b697021ba8188c906e559102daf083704df114106faa4e1e9",
# "expires_at":"2026-05-07T15:36:41.146999913Z"
#}

# https://httpbin.org/status/200
# https://webhook.site/19b83c6a-6f50-484b-8b03-5f5ee0f44d4a
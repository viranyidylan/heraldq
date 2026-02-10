# Railway Deployment Guide

## Overview
This project has two services that need to be deployed separately:
- **API**: Axum web server with webhook endpoints
- **Worker**: Graphile Worker background job processor

Both services connect to the same PostgreSQL database.

## Prerequisites
1. Push your code to a GitHub repository
2. Have a Railway account linked to GitHub
3. Have a PostgreSQL database (can create one on Railway)

## Deployment Steps

### 1. Set Up Database (if not already done)
1. Go to [Railway](https://railway.app/)
2. Create a new project
3. Add a PostgreSQL database service
4. Note the `DATABASE_URL` connection string from the database settings

### 2. Deploy the API Service
1. In your Railway project, click **New Service**
2. Select **Deploy from GitHub repo**
3. Choose your repository `test_graphile`
4. Railway will detect the `railway.toml` and `Dockerfile.api`
5. In the service settings:
   - **Service Name**: `api`
   - **Root Directory**: `/` (leave as default)
   - Railway will automatically use `Dockerfile.api` based on `railway.toml`
6. Add environment variables:
   - `DATABASE_URL`: (copy from your PostgreSQL service or link the service)
7. Deploy the service

### 3. Expose the API Publicly
1. Go to your API service settings
2. Navigate to **Networking** tab
3. Click **Generate Domain** to create a public URL
4. Your API will be accessible at `https://<your-service>.railway.app`

### 4. Deploy the Worker Service
1. In your Railway project, click **New Service**
2. Select **Deploy from GitHub repo**
3. Choose the same repository `test_graphile`
4. In the service settings:
   - **Service Name**: `worker`
   - **Root Directory**: `/` (leave as default)
   - Railway will automatically use `Dockerfile.worker` based on `railway.toml`
5. Add environment variables:
   - `DATABASE_URL`: (copy from your PostgreSQL service or link the service)
6. Deploy the service
7. **Important**: Worker does NOT need a public domain - skip the networking setup

### 5. Verify Deployment

#### Check API:
```bash
curl https://<your-api-domain>.railway.app/
# Should return: "Hi Dylan"
```

#### Check Logs:
- **API logs** should show:
  ```
  [api] >>> starting api
  App listening at address: 0.0.0.0:8080
  ```
  
- **Worker logs** should show:
  ```
  [worker] >>> Spinning up
  [worker] >>> database connection established
  [worker] >>> graphile configured
  [worker] >>> starting worker...
  ```

### 6. Test the Webhook
```bash
curl -X POST https://<your-api-domain>.railway.app/webhook \
  -H "Content-Type: application/json" \
  -d '{"your": "data"}'
```

Check the worker logs to see it processing the job.

## Alternative: Manual Service Creation

If Railway doesn't automatically separate the services:

1. Create two separate services manually
2. For each service, go to **Settings → Build**
3. Set **Dockerfile Path**:
   - API service: `Dockerfile.api`
   - Worker service: `Dockerfile.worker`

## Troubleshooting

### 502 Bad Gateway
- Ensure the API is binding to `0.0.0.0` and using Railway's `PORT` env var (already configured)
- Check API logs for startup errors
- Verify `DATABASE_URL` is set correctly

### Worker Not Running
- Check worker logs for connection errors
- Verify `DATABASE_URL` matches the API's database
- Ensure the graphile_worker schema and tables exist in the database

### Database Connection Issues
- Make sure both services have `DATABASE_URL` environment variable
- The connection string should include all necessary credentials
- Consider using Railway's service linking feature

## Project Structure
```
.
├── Dockerfile.api          # API service Dockerfile
├── Dockerfile.worker       # Worker service Dockerfile
├── railway.toml           # Railway configuration for both services
├── Cargo.toml
└── src/
    ├── lib.rs
    └── bin/
        ├── api.rs         # API binary
        └── worker.rs      # Worker binary
```

## Environment Variables
Both services need:
- `DATABASE_URL`: PostgreSQL connection string

API also uses:
- `PORT`: Automatically set by Railway (defaults to 8080 in code)

## Notes
- The Dockerfiles use multi-stage builds with `cargo-chef` for faster builds
- Dependencies are cached between builds
- Railway automatically rebuilds when you push to GitHub
- You can view real-time logs for both services in the Railway dashboard

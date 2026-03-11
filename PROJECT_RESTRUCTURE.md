# Project Restructure: Module Organization

## Why This Exists

`lib.rs` is currently an implementation file — it holds constants, models, and business logic. `api.rs` mixes an extractor, shared state, and route handlers. As DLQ, status endpoints, rate limiting, and observability land, both files will balloon and become hard to reason about.

The restructure gives each file **one reason to change**, following the natural seams already in the domain.

---

## Core Principle: Single Responsibility Per File

Ask yourself: *what would cause this file to change?*

Right now `lib.rs` changes if you:
- Change the `WebhookRequest` model
- Change retry or dispatch behavior
- Change the job struct
- Add DLQ logic

Four reasons. One file. That's the smell. Every decision in this restructure is about reducing that number to one per file.

---

## Core Philosophy

**`lib.rs` is a table of contents, not a chapter.**
It should only declare modules (`mod webhook; mod auth;`) and control re-exports (`pub use webhook::WebhookRequest;`). No logic lives here.

**Binaries are assembly scripts, not implementation files.**
`api.rs` and `worker.rs` should read as a startup recipe: load config → connect DB → compose app → serve. Business logic belongs in the library, not the binary. Code in a binary is untestable and unreusable.

**Single crate, not a workspace.**
The two binaries share nearly all dependencies and domain types. Workspace overhead isn't justified until you have independent release cycles or materially divergent dependency graphs.

---

## Target File Tree

```
src/
├── lib.rs                    # module declarations + pub re-exports only
├── config.rs                 # AppConfig struct, Config::from_env() -> Result<Config>
├── error.rs                  # AppError enum, IntoResponse impl
├── db/
│   ├── mod.rs                # create_pool(), run_migrations()
│   └── dlq.rs                # DLQ insert query
├── webhook/
│   ├── mod.rs                # re-exports
│   ├── model.rs              # WebhookRequest, SendWebhook structs
│   └── dispatch.rs           # try_dispatch_webhook, TaskHandler impl
├── auth/
│   └── mod.rs                # ValidApiKey extractor
└── bin/
    ├── api.rs                 # thin: config → pool → router → serve
    └── worker.rs              # thin: config → pool → worker.run()
```

---

## Migration Steps

Each step ends with `cargo check` passing before moving on. Never skip this.

---

### Step 1: `config.rs`

**One reason to change:** new configuration options are added.

- Create `AppConfig` struct with at minimum `database_url: String` and `schema: String`
- Implement `Config::from_env() -> Result<Config, ...>`
- Both binaries call `Config::from_env()` at startup; hardcoded values disappear

**Why first:** zero dependencies on anything else in the codebase. Safest first move, and almost every module added after this will take a `&Config`.

---

### Step 2: `error.rs`

**One reason to change:** a new failure mode is introduced.

- Add `thiserror` to `Cargo.toml`
- Define `AppError` enum: `Database(sqlx::Error)`, `Unauthorized`, `JobEnqueue`, `NotFound`
- Implement `IntoResponse` for `AppError` — each variant controls its own status code and response body
- Implement `From<sqlx::Error> for AppError` so `.await?` auto-converts DB errors
- Handlers now return `Result<impl IntoResponse, AppError>` and use `?` throughout

**Why before new features:** every upcoming feature (DLQ writes, status endpoint, rate limiting) introduces new failure modes. Without a unified error type, each one gets another ad-hoc `StatusCode::INTERNAL_SERVER_ERROR`. With it, errors are structured, loggable, and propagate cleanly.

---

### Step 3: `auth/mod.rs`

**One reason to change:** authentication logic changes.

- Move `ValidApiKey` extractor out of `api.rs` into this module
- Update it to return `AppError::Unauthorized` instead of bare `StatusCode`
- `api.rs` imports it; no behavior changes, only location

**Why now:** `AppError` exists (Step 2), so the extractor can return it. `api.rs` shrinks meaningfully and becomes easier to read before you add new routes in upcoming sessions.

---

### Step 4: `webhook/model.rs` + `webhook/dispatch.rs`

**`model.rs` — one reason to change:** the shape of webhook data changes.
**`dispatch.rs` — one reason to change:** dispatch behavior changes.

- Move `WebhookRequest` + `SendWebhook` structs to `model.rs`
- Move `try_dispatch_webhook` + the `TaskHandler` impl to `dispatch.rs`
- `webhook/mod.rs` re-exports both
- `lib.rs` re-exports from `webhook`

**Why split model from dispatch:** the model is imported by API handlers, the worker, and eventually the DLQ writer. The dispatch logic is only relevant to the worker's execution path. Keeping them separate means a schema change doesn't visually mix with a retry logic change.

---

### Step 5: `db/mod.rs`

**One reason to change:** how the DB connection is established changes.

- Move `PgPoolOptions` setup into `db::create_pool(&config) -> Result<Pool<Postgres>>`
- Move `sqlx::migrate!` call into `db::run_migrations(&pool)`
- Both binaries use `db::create_pool`; duplication is removed

**Why now:** once this lands, both binaries are essentially thin. Step 6 (DLQ) then adds a query function into a clean module rather than into an already-messy file.

---

### Step 6: `db/dlq.rs` + wire DLQ writes

**One reason to change:** the DLQ schema or write logic changes.

- Uncomment (or add a new migration for) the `dead_letter_queue` table
- Add `db::dlq::insert(pool, record) -> Result<(), AppError>` query function
- Wire into `dispatch.rs` on the 4xx permanent-fail path and the max-attempts-exhausted path

**Why last in the restructure:** this is the first real new feature. Every previous step was structural cleanup that makes this step clean to implement. Without the restructure, DLQ logic would land in `lib.rs` alongside models and constants.

---

## Key Patterns

**`AppError` + `IntoResponse`**
Handlers return `Result<impl IntoResponse, AppError>`. You use `?` everywhere. No more match blocks that convert errors into `INTERNAL_SERVER_ERROR` and swallow the details.

**`AppState` does not belong in `api.rs`**
It belongs in `src/app_state.rs` or exported from `lib.rs`. The binary startup sequence and the shared application state have different reasons to change.

**What a thin binary looks like**

```rust
// bin/api.rs after restructuring
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_env()?;
    let pool = db::create_pool(&config).await?;
    db::run_migrations(&pool).await?;
    let state = AppState::new(pool, &config);
    let app = routes::build(state);
    let listener = TcpListener::bind(format!("0.0.0.0:{}", config.port)).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
```

No domain logic. Readable in under a minute with no prior context.

---

## Verification Checkpoints

| After step | Verify |
|---|---|
| Each step | `cargo check` passes |
| Step 3 | `cargo run --bin api` → `GET /ping` returns `pong`, `POST /webhook` with valid key returns 202 |
| Step 5 | Both binaries start cleanly; worker polls and dispatches a test job |
| Step 6 | Enqueue a job to an invalid URL; row appears in `dead_letter_queue` after max attempts |

use rand::RngExt;
use rand::distr::Alphanumeric;
use sliq::{config::AppConfig, db};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    let config = AppConfig::from_env()?;
    let pg_pool = db::create_pool(&config).await?;

    for _ in 0..n {
        let suffix: String = rand::rng()
            .sample_iter(Alphanumeric)
            .take(8)
            .map(char::from)
            .collect();

        let code = format!("BETA_{suffix}");

        sqlx::query!("INSERT INTO beta_codes (code) VALUES ($1)", code)
            .execute(&pg_pool)
            .await?;

        println!("{code}");
    }

    println!("Seeded {n} codes");
    Ok(())
}
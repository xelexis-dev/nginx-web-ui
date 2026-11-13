//! Isolated migration verifier / operator entry point. The API calls the same function at boot.
use deadpool_postgres::{Manager, Pool};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = std::env::var("DATABASE_URL")?.parse::<tokio_postgres::Config>()?;
    let pool = Pool::builder(Manager::new(config, tokio_postgres::NoTls)).max_size(1).build()?;
    let directory = std::env::var("MIGRATIONS_DIR")?;
    let count = nwui_api::migrations::run(&pool, std::path::Path::new(&directory)).await?;
    println!("Applied {count} migrations");
    Ok(())
}

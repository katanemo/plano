pub mod models;
pub mod queries;

use deadpool_postgres::{Config, Pool, Runtime};
use tokio_postgres::NoTls;
use tracing::info;

#[derive(Clone)]
pub struct DbPool {
    pool: Pool,
}

impl DbPool {
    pub fn new(database_url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let mut cfg = Config::new();
        cfg.url = Some(database_url.to_string());
        let pool = cfg.create_pool(Some(Runtime::Tokio1), NoTls)?;
        info!("database connection pool created");
        Ok(Self { pool })
    }

    pub fn inner(&self) -> &Pool {
        &self.pool
    }

    pub async fn get_client(
        &self,
    ) -> Result<deadpool_postgres::Client, deadpool_postgres::PoolError> {
        self.pool.get().await
    }
}

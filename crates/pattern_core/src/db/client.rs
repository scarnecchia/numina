//! Direct SurrealDB client implementation

use crate::db::{DatabaseConfig, DatabaseError, Result};
use std::sync::LazyLock;
use surrealdb::engine::any;

use surrealdb::{Connection, Surreal};

/// Global database instance using the LazyLock pattern from SurrealDB docs
pub static DB: LazyLock<Surreal<surrealdb::engine::any::Any>> = LazyLock::new(Surreal::init);

/// Create a new database instance for testing
pub async fn create_test_db() -> Result<Surreal<any::Any>> {
    let db = any::connect("memory").await.unwrap();
    // For embedded mode, we need to select a namespace and database
    db.use_ns("pattern")
        .use_db("pattern")
        .await
        .map_err(|e| DatabaseError::ConnectionFailed(e))?;

    // Run migrations
    use crate::db::migration::MigrationRunner;
    MigrationRunner::run(&db).await?;
    Ok(db)
}

/// Initialize a database instance (non-global) for testing
pub async fn init_db_instance<C: Connection>(
    config: DatabaseConfig,
) -> Result<Surreal<impl Connection>> {
    match config {
        DatabaseConfig::Embedded { path, .. } => {
            let path = if path.is_empty() {
                "memory".to_string()
            } else {
                // Ensure parent directory exists for file-based storage
                if let Some(parent) = std::path::Path::new(&path).parent() {
                    if !parent.exists() {
                        std::fs::create_dir_all(parent).map_err(|e| {
                            DatabaseError::Other(format!(
                                "Failed to create database directory: {}",
                                e
                            ))
                        })?;
                    }
                }
                format!("surrealkv://{}", path)
            };
            // Connect to the embedded database
            let db = any::connect(path)
                .await
                .map_err(|e| DatabaseError::ConnectionFailed(e))?;

            // For embedded mode, we need to select a namespace and database
            db.use_ns("pattern")
                .use_db("pattern")
                .await
                .map_err(|e| DatabaseError::ConnectionFailed(e))?;

            // Run migrations
            use crate::db::migration::MigrationRunner;
            MigrationRunner::run(&db).await?;

            Ok(db)
        }
        #[cfg(feature = "surreal-remote")]
        DatabaseConfig::Remote {
            url,
            username,
            password,
            namespace,
            database,
        } => {
            // Connect to remote database
            use surrealdb::opt::auth::Root;

            let db = any::connect(url)
                .await
                .map_err(|e| DatabaseError::ConnectionFailed(e))?;

            // Authenticate if credentials provided
            if !username.is_none() && !password.is_none() {
                db.signin(Root {
                    username: &username.unwrap(),
                    password: &password.unwrap(),
                })
                .await
                .map_err(|e| DatabaseError::ConnectionFailed(e))?;
            }

            // Select namespace and database
            db.use_ns(namespace)
                .use_db(database)
                .await
                .map_err(|e| DatabaseError::ConnectionFailed(e))?;

            // Run migrations
            use crate::db::migration::MigrationRunner;
            MigrationRunner::run(&db).await?;

            Ok(db)
        }
    }
}

/// Initialize the database connection
pub async fn init_db(config: DatabaseConfig) -> Result<()> {
    match config {
        DatabaseConfig::Embedded { path, .. } => {
            let path = if path.is_empty() {
                "memory".to_string()
            } else {
                // Ensure parent directory exists for file-based storage
                if let Some(parent) = std::path::Path::new(&path).parent() {
                    if !parent.exists() {
                        std::fs::create_dir_all(parent).map_err(|e| {
                            DatabaseError::Other(format!(
                                "Failed to create database directory: {}",
                                e
                            ))
                        })?;
                    }
                }
                format!("surrealkv://{}", path)
            };
            // Connect to the embedded database
            match DB.connect(&path).await {
                Ok(_) => {}
                Err(surrealdb::Error::Api(surrealdb::error::Api::AlreadyConnected)) => {
                    // Already connected, that's fine for tests
                }
                Err(e) => return Err(DatabaseError::ConnectionFailed(e)),
            }

            // For embedded mode, we need to select a namespace and database
            DB.use_ns("pattern")
                .use_db("pattern")
                .await
                .map_err(|e| DatabaseError::ConnectionFailed(e))?;
        }
        #[cfg(feature = "surreal-remote")]
        DatabaseConfig::Remote {
            url,
            username,
            password,
            namespace,
            database,
        } => {
            // Connect to remote database
            use surrealdb::opt::auth::Root;

            // Connect handling AlreadyConnected error
            let connect_result = if url.starts_with("wss://") {
                DB.connect(url).await
            } else {
                DB.connect(url).await
            };

            match connect_result {
                Ok(_) => {}
                Err(surrealdb::Error::Api(surrealdb::error::Api::AlreadyConnected)) => {
                    // Already connected, that's fine
                }
                Err(e) => return Err(DatabaseError::ConnectionFailed(e)),
            }

            // Authenticate if credentials provided
            if let (Some(user), Some(pass)) = (username, password) {
                DB.signin(Root {
                    username: &user,
                    password: &pass,
                })
                .await
                .map_err(|e| DatabaseError::ConnectionFailed(e))?;
            }

            // Select namespace and database
            DB.use_ns(namespace)
                .use_db(database)
                .await
                .map_err(|e| DatabaseError::ConnectionFailed(e))?;
        }
    }

    // Initialize the schema
    crate::db::migration::MigrationRunner::run(&DB).await?;

    Ok(())
}

/// Check if the database is healthy
pub async fn health_check() -> Result<()> {
    DB.health()
        .await
        .map_err(|e| DatabaseError::ConnectionFailed(e))
}

///Get the database version
pub async fn version() -> Result<String> {
    DB.version()
        .await
        .map_err(|e| DatabaseError::QueryFailed(e))
        .map(|v| v.to_string())
}

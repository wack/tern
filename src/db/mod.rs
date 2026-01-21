pub mod compile;
pub mod diff;
pub mod history;
pub mod migrate;
pub mod model;
pub mod pglite;
pub mod query;
pub mod schema;
pub mod state;

use rustls::ClientConfig;
use tokio_postgres::Client;
use tokio_postgres_rustls::MakeRustlsConnect;

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum DbError {
    #[error("failed to connect to database: {0}")]
    Connection(#[from] tokio_postgres::Error),
}

/// Connects to a PostgreSQL database using the provided connection string.
///
/// Returns the connected client. The connection is automatically spawned
/// as a background task to drive it to completion.
///
/// # Example
///
/// ```ignore
/// let client = connect("postgres://user:pass@localhost/db").await?;
///
/// // Use the client...
/// let rows = client.query("SELECT 1", &[]).await?;
/// ```
pub async fn connect(connection_string: &str) -> Result<Client, DbError> {
    let tls_config = ClientConfig::builder()
        .with_root_certificates(root_certificates())
        .with_no_client_auth();

    let tls_connector = MakeRustlsConnect::new(tls_config);

    let (client, connection) = tokio_postgres::connect(connection_string, tls_connector).await?;

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            tracing::error!("database connection error: {}", e);
        }
    });

    Ok(client)
}

fn root_certificates() -> rustls::RootCertStore {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    roots
}

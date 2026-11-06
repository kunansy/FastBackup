use std::time::Duration;

use backup_core::{Error, Result};
use google_drive3::DriveHub;
use google_drive3::hyper_util::client::legacy::{Client, connect::HttpConnector};
use google_drive3::hyper_util::rt::TokioExecutor;
use google_drive3::yup_oauth2::{
    ServiceAccountAuthenticator, client::CustomHyperClientBuilder, parse_service_account_key,
};
use hyper_rustls::HttpsConnector;

pub(crate) type Hub = DriveHub<HttpsConnector<HttpConnector>>;

// Split by hyper across all resolved addresses, so an unreachable IP is skipped.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

fn build_connector() -> Result<HttpsConnector<HttpConnector>> {
    let mut http = HttpConnector::new();
    http.enforce_http(false);
    http.set_connect_timeout(Some(CONNECT_TIMEOUT));

    let connector = hyper_rustls::HttpsConnectorBuilder::new()
        .with_native_roots()
        .map_err(|e| Error::Storage(format!("Could not load native TLS roots: {e}")))?
        .https_only()
        .enable_http1()
        .enable_http2()
        .wrap_connector(http);
    Ok(connector)
}

pub(crate) async fn build_hub(creds: &str) -> Result<Hub> {
    let secret = parse_service_account_key(creds)
        .map_err(|e| Error::Config(format!("Could not parse drive creds: {e}")))?;

    let auth_client = Client::builder(TokioExecutor::new()).build(build_connector()?);
    let auth = ServiceAccountAuthenticator::with_client(
        secret,
        CustomHyperClientBuilder::from(auth_client),
    )
    .build()
    .await
    .map_err(|e| Error::Storage(format!("Could not build drive authenticator: {e}")))?;

    let client = Client::builder(TokioExecutor::new()).build(build_connector()?);
    Ok(DriveHub::new(client, auth))
}

/// Hub without auth that talks plain HTTP to `base`, for tests against a fake server.
#[cfg(test)]
pub(crate) fn test_hub(base: &str) -> Hub {
    let mut http = HttpConnector::new();
    http.enforce_http(false);
    let connector = hyper_rustls::HttpsConnectorBuilder::new()
        .with_native_roots()
        .expect("native roots")
        .https_or_http()
        .enable_http1()
        .wrap_connector(http);
    let client = Client::builder(TokioExecutor::new()).build(connector);

    let mut hub = DriveHub::new(client, StaticToken);
    hub.base_url(format!("{base}/drive/v3/"));
    hub.root_url(format!("{base}/"));
    hub
}

#[cfg(test)]
#[derive(Clone)]
struct StaticToken;

#[cfg(test)]
impl google_drive3::common::GetToken for StaticToken {
    fn get_token<'a>(
        &'a self,
        _scopes: &'a [&str],
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = std::result::Result<
                        Option<String>,
                        Box<dyn std::error::Error + Send + Sync>,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async { Ok(Some("test-token".to_string())) })
    }
}

use anyhow::Result;
use librespot::core::cache::Cache;
use librespot::core::config::SessionConfig;
use librespot::core::session::Session;
use librespot::discovery::Credentials;
use librespot::oauth::OAuthClientBuilder;

const SPOTIFY_CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";
const SPOTIFY_REDIRECT_URI: &str = "http://127.0.0.1:8898/login";

#[tracing::instrument(name = "create_session", level = "debug")]
pub async fn create_session() -> Result<Session> {
    let session_config = SessionConfig::default();

    let credentials_path = crate::utils::get_dot_path()?;

    let cache = Cache::new(
        Some(credentials_path.clone()),
        None,
        Some(credentials_path),
        None,
    )?;

    let credentials = match cache.credentials() {
        Some(credentials) => {
            tracing::info!("Using cached credentials");
            credentials
        }
        None => {
            tracing::info!("No cached credentials found, starting OAuth flow");
            load_credentials()?
        }
    };

    let session = Session::new(session_config, Some(cache));
    session.connect(credentials, true).await?;

    Ok(session)
}

pub fn load_credentials() -> Result<Credentials> {
    let client = OAuthClientBuilder::new(SPOTIFY_CLIENT_ID, SPOTIFY_REDIRECT_URI, vec!["streaming"]).build()?;
    let token = match client.get_access_token() {
        Ok(token) => token,
        Err(e) => return Err(e.into()),
    };
    Ok(Credentials::with_access_token(token.access_token))
}

use services::mmb::lastfm::{LASTFM_API_KEY, LASTFM_API_SECRET};

mod devices;
mod library;
mod media;
mod playback;
mod services;
mod settings;
mod ui;
mod util;

#[async_std::main]
async fn main() {
    tracing_subscriber::fmt::init();

    tracing::info!("Starting application");

    if LASTFM_API_KEY.is_none() || LASTFM_API_SECRET.is_none() {
        tracing::warn!("Binary not compiled with LastFM support, set LASTFM_API_KEY and LASTFM_API_SECRET at compile time to enable");
    }

    crate::ui::app::run().await;
}

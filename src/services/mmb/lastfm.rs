use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use client::LastFMClient;
use tracing::{debug, warn};

use crate::{media::metadata::Metadata, playback::thread::PlaybackState};

use super::MediaMetadataBroadcastService;

pub mod client;
mod requests;
pub mod types;
mod util;

pub const LASTFM_API_KEY: Option<&'static str> = option_env!("LASTFM_API_KEY");
pub const LASTFM_API_SECRET: Option<&'static str> = option_env!("LASTFM_API_SECRET");

pub struct LastFM {
    client: LastFMClient,
    start_timestamp: Option<DateTime<Utc>>,
    accumulated_time: u64,
    duration: u64,
    metadata: Option<Arc<Metadata>>,
    last_postion: u64,
    has_scrobbled: bool,
}

impl LastFM {
    pub fn new(client: LastFMClient) -> Self {
        LastFM {
            client,
            start_timestamp: None,
            accumulated_time: 0,
            metadata: None,
            duration: 0,
            last_postion: 0,
            has_scrobbled: true,
        }
    }
}

#[async_trait]
impl MediaMetadataBroadcastService for LastFM {
    async fn new_track(&mut self, _: String) {
        self.start_timestamp = Some(chrono::offset::Utc::now());
        self.accumulated_time = 0;
        self.last_postion = 0;
        self.has_scrobbled = false;
    }

    async fn metadata_recieved(&mut self, info: Arc<Metadata>) {
        if let (Some(artist), Some(track)) = (info.artist.clone(), info.name.clone()) {
            if let Err(e) = self
                .client
                .now_playing(artist, track, info.album.clone(), None)
                .await
            {
                warn!("Could not set now playing: {}", e)
            }
        }

        self.metadata = Some(info);
    }

    async fn state_changed(&mut self, _: PlaybackState) {}

    async fn position_changed(&mut self, position: u64) {
        if position < self.last_postion + 2 {
            self.accumulated_time += position - self.last_postion;
        }

        self.last_postion = position;

        if self.duration >= 30
            && (self.accumulated_time > self.duration / 2 || self.accumulated_time > 240)
            && !self.has_scrobbled
        {
            if let Some(info) = &self.metadata {
                debug!("attempting scrobble");
                if let (Some(artist), Some(track)) = (info.artist.clone(), info.name.clone()) {
                    self.has_scrobbled = true;
                    if let Err(e) = self
                        .client
                        .scrobble(
                            artist,
                            track,
                            self.start_timestamp.unwrap(),
                            info.album.clone(),
                            None,
                        )
                        .await
                    {
                        warn!("Could not scrobble: {}", e)
                    }
                }
            }
        }
    }

    async fn duration_changed(&mut self, duration: u64) {
        self.duration = duration;
    }
}
use std::{
    collections::VecDeque,
    fs::{File, OpenOptions},
    sync::{Arc, RwLock},
};

use ahash::AHashMap;
use async_std::sync::Mutex;
use gpui::{App, AppContext, Entity, EventEmitter, Global, RenderImage};
use tracing::{debug, error, warn};

use crate::{
    library::scan::ScanEvent,
    media::metadata::Metadata,
    playback::{
        queue::{QueueItemData, QueueItemUIData},
        thread::PlaybackState,
    },
    services::mmb::{
        lastfm::{client::LastFMClient, types::Session, LastFM, LASTFM_API_KEY, LASTFM_API_SECRET},
        MediaMetadataBroadcastService,
    },
    ui::{app::get_dirs, library::ViewSwitchMessage},
};

// yes this looks a little silly
impl EventEmitter<Metadata> for Metadata {}

#[derive(Debug, PartialEq, Clone)]
pub struct ImageEvent(pub Box<[u8]>);

impl EventEmitter<ImageEvent> for Option<Arc<RenderImage>> {}

#[derive(Clone)]
pub enum LastFMState {
    Disconnected,
    AwaitingFinalization(String),
    Connected(Session),
}

impl EventEmitter<Session> for LastFMState {}

pub struct Models {
    pub metadata: Entity<Metadata>,
    pub albumart: Entity<Option<Arc<RenderImage>>>,
    pub queue: Entity<Queue>,
    pub scan_state: Entity<ScanEvent>,
    pub mmbs: Entity<MMBSList>,
    pub lastfm: Entity<LastFMState>,
    pub switcher_model: Entity<VecDeque<ViewSwitchMessage>>,
}

impl Global for Models {}

#[derive(Clone)]
pub struct PlaybackInfo {
    pub position: Entity<u64>,
    pub duration: Entity<u64>,
    pub playback_state: Entity<PlaybackState>,
    pub current_track: Entity<Option<String>>,
    pub shuffling: Entity<bool>,
    pub volume: Entity<f64>,
}

impl Global for PlaybackInfo {}

// pub struct ImageTransfer(pub ImageType, pub Arc<RenderImage>);
// pub struct TransferDummy;

// impl EventEmitter<ImageTransfer> for TransferDummy {}

#[derive(Debug, Clone)]
pub struct Queue {
    pub data: Arc<RwLock<Vec<QueueItemData>>>,
    pub position: usize,
}

impl EventEmitter<(String, QueueItemUIData)> for Queue {}

#[derive(Clone)]
pub struct MMBSList(pub AHashMap<String, Arc<Mutex<dyn MediaMetadataBroadcastService>>>);

#[derive(Clone)]
pub enum MMBSEvent {
    NewTrack(String),
    MetadataRecieved(Arc<Metadata>),
    StateChanged(PlaybackState),
    PositionChanged(u64),
    DurationChanged(u64),
}

impl EventEmitter<MMBSEvent> for MMBSList {}

pub fn build_models(cx: &mut App, queue: Queue) {
    debug!("Building models");
    let metadata: Entity<Metadata> = cx.new(|_| Metadata::default());
    let albumart: Entity<Option<Arc<RenderImage>>> = cx.new(|_| None);
    let queue: Entity<Queue> = cx.new(move |_| queue);
    let scan_state: Entity<ScanEvent> = cx.new(|_| ScanEvent::ScanCompleteIdle);
    let mmbs: Entity<MMBSList> = cx.new(|_| MMBSList(AHashMap::new()));
    let lastfm: Entity<LastFMState> = cx.new(|cx| {
        let dirs = get_dirs();
        let directory = dirs.data_dir().to_path_buf();
        let path = directory.join("lastfm.json");

        if let Ok(file) = File::open(path) {
            let reader = std::io::BufReader::new(file);

            if let Ok(session) = serde_json::from_reader::<std::io::BufReader<File>, Session>(reader) {
                create_last_fm_mmbs(cx, &mmbs, session.key.clone());
                LastFMState::Connected(session)
            } else {
                error!("The last.fm session information is stored on disk but the file could not be opened.");
                warn!("You will not be logged in to last.fm.");
                LastFMState::Disconnected
            }
        } else {
            LastFMState::Disconnected
        }
    });

    // cx.subscribe(&albumart, |_, ev, cx| {
    //     let img = ev.0.clone();
    //     cx.global::<GPUIDataInterface>().decode_image(
    //         img,
    //         ImageType::CurrentAlbumArt,
    //         ImageLayout::BGR,
    //         true,
    //     );
    // })
    // .detach();

    let mmbs_clone = mmbs.clone();

    cx.subscribe(&lastfm, move |m, ev, cx| {
        let session_clone = ev.clone();
        create_last_fm_mmbs(cx, &mmbs_clone, session_clone.key.clone());
        m.update(cx, |m, cx| {
            *m = LastFMState::Connected(session_clone);
            cx.notify();
        });

        let dirs = get_dirs();
        let directory = dirs.data_dir().to_path_buf();
        let path = directory.join("lastfm.json");
        let file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .create(true)
            .open(path);

        if let Ok(file) = file {
            let writer = std::io::BufWriter::new(file);
            if serde_json::to_writer_pretty(writer, ev).is_err() {
                error!("Tried to write lastfm settings but could not write to file!");
                error!("You will have to sign in again when the application is next started.");
            }
        } else {
            error!("Tried to write lastfm settings but could not open file!");
            error!("You will have to sign in again when the application is next started.");
        }
    })
    .detach();

    cx.subscribe(&mmbs, |m, ev, cx| {
        let list = m.read(cx);

        // cloning actually is neccesary because of the async move closure
        #[allow(clippy::unnecessary_to_owned)]
        for mmbs in list.0.values().cloned() {
            let ev = ev.clone();
            cx.spawn(|_| async move {
                let mut borrow = mmbs.lock().await;
                match ev {
                    MMBSEvent::NewTrack(path) => borrow.new_track(path),
                    MMBSEvent::MetadataRecieved(metadata) => borrow.metadata_recieved(metadata),
                    MMBSEvent::StateChanged(state) => borrow.state_changed(state),
                    MMBSEvent::PositionChanged(position) => borrow.position_changed(position),
                    MMBSEvent::DurationChanged(duration) => borrow.duration_changed(duration),
                }
                .await;
            })
            .detach();
        }
    })
    .detach();

    let switcher_model = cx.new(|_| {
        let mut deque = VecDeque::new();
        deque.push_back(ViewSwitchMessage::Albums);
        deque
    });

    cx.set_global(Models {
        metadata,
        albumart,
        queue,
        scan_state,
        mmbs,
        lastfm,
        switcher_model,
    });

    let position: Entity<u64> = cx.new(|_| 0);
    let duration: Entity<u64> = cx.new(|_| 0);
    let playback_state: Entity<PlaybackState> = cx.new(|_| PlaybackState::Stopped);
    let current_track: Entity<Option<String>> = cx.new(|_| None);
    let shuffling: Entity<bool> = cx.new(|_| false);
    let volume: Entity<f64> = cx.new(|_| 1.0);

    cx.set_global(PlaybackInfo {
        position,
        duration,
        playback_state,
        current_track,
        shuffling,
        volume,
    });
}

pub fn create_last_fm_mmbs(cx: &mut App, mmbs_list: &Entity<MMBSList>, session: String) {
    if let (Some(key), Some(secret)) = (LASTFM_API_KEY, LASTFM_API_SECRET) {
        let mut client = LastFMClient::new(key.to_string(), secret);
        client.set_session(session);
        let mmbs = LastFM::new(client);
        mmbs_list.update(cx, |m, _| {
            m.0.insert("lastfm".to_string(), Arc::new(Mutex::new(mmbs)));
        })
    }
}

use std::{
    fs::{self, File},
    io::{BufReader, Cursor, Write},
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, SystemTime},
};

use ahash::AHashMap;
use async_std::task;
use globwalk::GlobWalkerBuilder;
use gpui::{App, Global};
use image::{codecs::jpeg::JpegEncoder, imageops::thumbnail, EncodableLayout};
use sqlx::SqlitePool;
use tracing::{debug, error, info, warn};

use crate::{
    media::{
        builtin::symphonia::SymphoniaProvider,
        metadata::Metadata,
        traits::{MediaPlugin, MediaProvider},
    },
    settings::scan::ScanSettings,
    ui::models::Models,
};

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum ScanEvent {
    Cleaning,
    DiscoverProgress(u64),
    ScanProgress { current: u64, total: u64 },
    ScanCompleteWatching,
    ScanCompleteIdle,
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum ScanCommand {
    Scan,
    Stop,
}

pub struct ScanInterface {
    events_rx: Option<mpsc::Receiver<ScanEvent>>,
    command_tx: mpsc::Sender<ScanCommand>,
}

impl ScanInterface {
    pub(self) fn new(
        events_rx: Option<mpsc::Receiver<ScanEvent>>,
        command_tx: mpsc::Sender<ScanCommand>,
    ) -> Self {
        ScanInterface {
            events_rx,
            command_tx,
        }
    }

    pub fn scan(&self) {
        self.command_tx
            .send(ScanCommand::Scan)
            .expect("could not send tx");
    }

    pub fn stop(&self) {
        self.command_tx
            .send(ScanCommand::Stop)
            .expect("could not send tx");
    }

    pub fn start_broadcast(&mut self, cx: &mut App) {
        let mut events_rx = None;
        std::mem::swap(&mut self.events_rx, &mut events_rx);

        let state_model = cx.global::<Models>().scan_state.clone();

        let Some(events_rx) = events_rx else {
            return;
        };
        cx.spawn(|mut cx| async move {
            loop {
                while let Ok(event) = events_rx.try_recv() {
                    state_model
                        .update(&mut cx, |m, cx| {
                            *m = event;
                            cx.notify()
                        })
                        .expect("failed to update scan state model");
                }

                cx.background_executor()
                    .timer(Duration::from_millis(10))
                    .await;
            }
        })
        .detach();
    }
}

impl Global for ScanInterface {}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum ScanState {
    Idle,
    Cleanup,
    Discovering,
    Scanning,
}

pub struct ScanThread {
    event_tx: mpsc::Sender<ScanEvent>,
    command_rx: mpsc::Receiver<ScanCommand>,
    pool: SqlitePool,
    scan_settings: ScanSettings,
    visited: Vec<PathBuf>,
    discovered: Vec<PathBuf>,
    to_process: Vec<PathBuf>,
    scan_state: ScanState,
    provider_table: Vec<(&'static [&'static str], Box<dyn MediaProvider>)>,
    scan_record: AHashMap<PathBuf, u64>,
    scan_record_path: Option<PathBuf>,
    scanned: u64,
    discovered_total: u64,
}

fn build_provider_table() -> Vec<(&'static [&'static str], Box<dyn MediaProvider>)> {
    // TODO: dynamic plugin loading
    vec![(
        SymphoniaProvider::SUPPORTED_EXTENSIONS,
        Box::new(SymphoniaProvider::default()),
    )]
}

fn retrieve_base_paths() -> Vec<PathBuf> {
    // TODO: user-defined base paths
    // TODO: we should also probably check if these directories exist
    let system_music = directories::UserDirs::new()
        .unwrap()
        .audio_dir()
        .unwrap()
        .to_path_buf();

    vec![system_music]
}

fn file_is_scannable_with_provider(path: &Path, exts: &&[&str]) -> bool {
    for extension in exts.iter() {
        if let Some(ext) = path.extension() {
            if ext == *extension {
                return true;
            }
        }
    }

    false
}

type FileInformation = (Metadata, u64, Option<Box<[u8]>>);

// We don't care about the error message. If the file can't be scanned, we just ignore it.
// TODO: it might be worth logging why the file couldn't be scanned (for plugin development)
fn scan_file_with_provider(
    path: &PathBuf,
    provider: &mut Box<dyn MediaProvider>,
) -> Result<FileInformation, ()> {
    let src = std::fs::File::open(path).map_err(|_| ())?;
    provider.open(src, None).map_err(|_| ())?;
    provider.start_playback().map_err(|_| ())?;
    let metadata = provider.read_metadata().cloned().map_err(|_| ())?;
    let image = provider.read_image().map_err(|_| ())?;
    let len = provider.duration_secs().map_err(|_| ())?;
    provider.close().map_err(|_| ())?;
    Ok((metadata, len, image))
}

// Returns the first image (cover/front/folder.jpeg/png/jpeg) in the track's containing folder
// Album art can be named anything, but this pattern is convention and the least likely to return a false positive
fn scan_path_for_album_art(path: &Path) -> Option<Box<[u8]>> {
    let glob = GlobWalkerBuilder::from_patterns(
        path.parent().unwrap(),
        &["{folder,cover,front}.{jpg,jpeg,png}"],
    )
    .case_insensitive(true)
    .max_depth(1)
    .build()
    .expect("Failed to build album art glob")
    .into_iter()
    .filter_map(|e| e.ok());

    for entry in glob {
        if let Ok(bytes) = fs::read(entry.path()) {
            return Some(bytes.into_boxed_slice());
        }
    }
    return None;
}

impl ScanThread {
    pub fn start(pool: SqlitePool, settings: ScanSettings) -> ScanInterface {
        let (commands_tx, commands_rx) = std::sync::mpsc::channel();
        let (events_tx, events_rx) = std::sync::mpsc::channel();

        std::thread::Builder::new()
            .name("scanner".to_string())
            .spawn(move || {
                let mut thread = ScanThread {
                    event_tx: events_tx,
                    command_rx: commands_rx,
                    pool,
                    visited: Vec::new(),
                    discovered: Vec::new(),
                    to_process: Vec::new(),
                    scan_state: ScanState::Idle,
                    provider_table: build_provider_table(),
                    scan_settings: settings,
                    scan_record: AHashMap::new(),
                    scan_record_path: None,
                    scanned: 0,
                    discovered_total: 0,
                };

                thread.run();
            })
            .expect("could not start playback thread");

        ScanInterface::new(Some(events_rx), commands_tx)
    }

    fn run(&mut self) {
        let dirs = directories::ProjectDirs::from("me", "william341", "muzak")
            .expect("couldn't find project dirs");
        let directory = dirs.data_dir();
        if !directory.exists() {
            fs::create_dir(directory).expect("couldn't create data directory");
        }
        let file_path = directory.join("scan_record.json");

        if file_path.exists() {
            let file = File::open(&file_path);

            let Ok(file) = file else {
                return;
            };
            let reader = BufReader::new(file);

            match serde_json::from_reader(reader) {
                Ok(scan_record) => {
                    self.scan_record = scan_record;
                }
                Err(e) => {
                    error!("could not read scan record: {:?}", e);
                    error!("scanning will be slow until the scan record is rebuilt");
                }
            }
        }

        self.scan_record_path = Some(file_path);

        loop {
            self.read_commands();

            // TODO: clear out old files if they've been deleted or moved
            // TODO: connect to user interface to display progress
            // TODO: start file watcher to update db automatically when files are added or removed
            match self.scan_state {
                ScanState::Idle => {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                ScanState::Cleanup => {
                    self.cleanup();
                }
                ScanState::Discovering => {
                    self.discover();
                }
                ScanState::Scanning => {
                    self.scan();
                }
            }
        }
    }

    fn read_commands(&mut self) {
        while let Ok(command) = self.command_rx.try_recv() {
            match command {
                ScanCommand::Scan => {
                    if self.scan_state == ScanState::Idle {
                        self.discovered = self.scan_settings.paths.clone();
                        self.scan_state = ScanState::Cleanup;
                        self.scanned = 0;
                        self.discovered_total = 0;
                        self.event_tx
                            .send(ScanEvent::Cleaning)
                            .expect("could not send scan started event");
                    }
                }
                ScanCommand::Stop => {
                    self.scan_state = ScanState::Idle;
                    self.visited.clear();
                    self.discovered.clear();
                    self.to_process.clear();
                }
            }
        }

        if self.scan_state == ScanState::Discovering {
            self.discover();
        } else if self.scan_state == ScanState::Scanning {
            self.scan();
        } else {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    fn file_is_scannable(&mut self, path: &PathBuf) -> bool {
        let timestamp = match fs::metadata(path) {
            Ok(metadata) => metadata
                .modified()
                .unwrap()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            Err(_) => return false,
        };

        for (exts, _) in self.provider_table.iter() {
            let x = file_is_scannable_with_provider(path, exts);

            if !x {
                continue;
            }
            if let Some(last_scan) = self.scan_record.get(path) {
                if *last_scan == timestamp {
                    return false;
                }
            }

            self.scan_record.insert(path.clone(), timestamp);
            return true;
        }

        false
    }

    fn discover(&mut self) {
        if self.discovered.is_empty() {
            self.scan_state = ScanState::Scanning;
            return;
        }

        let path = self.discovered.pop().unwrap();

        if self.visited.contains(&path) {
            return;
        }

        let paths = fs::read_dir(&path).unwrap();

        for paths in paths {
            // TODO: handle errors
            // this might be slower than just reading the path directly but this prevents loops
            let path = paths.unwrap().path().canonicalize().unwrap();
            if path.is_dir() {
                self.discovered.push(path);
            } else if self.file_is_scannable(&path) {
                self.to_process.push(path);

                self.discovered_total += 1;

                if self.discovered_total % 20 == 0 {
                    self.event_tx
                        .send(ScanEvent::DiscoverProgress(self.discovered_total))
                        .expect("could not send discovered event");
                }
            }
        }

        self.visited.push(path.clone());
    }

    async fn insert_artist(&self, metadata: &Metadata) -> Option<i64> {
        let artist = metadata.album_artist.clone().or(metadata.artist.clone());

        let artist = artist?;
        let result: Result<(i64,), sqlx::Error> =
            sqlx::query_as(include_str!("../../queries/scan/create_artist.sql"))
                .bind(&artist)
                .bind(metadata.artist_sort.as_ref().unwrap_or(&artist))
                .fetch_one(&self.pool)
                .await;

        match result {
            Ok(v) => Some(v.0),
            Err(sqlx::Error::RowNotFound) => {
                let result: Result<(i64,), sqlx::Error> =
                    sqlx::query_as(include_str!("../../queries/scan/get_artist_id.sql"))
                        .bind(&artist)
                        .fetch_one(&self.pool)
                        .await;

                match result {
                    Ok(v) => Some(v.0),
                    Err(e) => {
                        error!("Database error while retriving artist: {:?}", e);
                        None
                    }
                }
            }
            Err(e) => {
                error!("Database error while creating artist: {:?}", e);
                None
            }
        }
    }

    async fn insert_album(
        &self,
        metadata: &Metadata,
        artist_id: Option<i64>,
        image: &Option<Box<[u8]>>,
    ) -> Option<i64> {
        let Some(album) = &metadata.album else {
            return None;
        };
        let result: Result<(i64,), sqlx::Error> =
            sqlx::query_as(include_str!("../../queries/scan/get_album_id.sql"))
                .bind(album)
                .fetch_one(&self.pool)
                .await;

        match result {
            Ok(v) => Some(v.0),
            Err(sqlx::Error::RowNotFound) => {
                let thumb = match image {
                    Some(image) => {
                        let decoded = image::ImageReader::new(Cursor::new(&image))
                            .with_guessed_format()
                            .ok()?
                            .decode()
                            .ok()?
                            .into_rgba8();

                        let thumb = thumbnail(&decoded, 70, 70);

                        let mut buf: Cursor<Vec<u8>> = Cursor::new(Vec::new());

                        thumb
                            .write_to(&mut buf, image::ImageFormat::Bmp)
                            .expect("i don't know how Cursor could fail");
                        buf.flush().expect("could not flush buffer");

                        Some(buf.get_mut().clone())
                    }
                    None => None,
                };

                let resized_image = match image {
                    Some(image) => {
                        let mut decoded = image::ImageReader::new(Cursor::new(&image))
                            .with_guessed_format()
                            .ok()?
                            .decode()
                            .ok()?
                            .into_rgb8();

                        if decoded.dimensions().0 <= 1024 || decoded.dimensions().1 <= 1024 {
                            Some(image.clone().to_vec())
                        } else {
                            decoded = image::imageops::resize(
                                &decoded,
                                1024,
                                1024,
                                image::imageops::FilterType::Lanczos3,
                            );
                            let mut buf: Cursor<Vec<u8>> = Cursor::new(Vec::new());
                            let mut encoder = JpegEncoder::new_with_quality(&mut buf, 70);

                            encoder
                                .encode(
                                    decoded.as_bytes(),
                                    decoded.width(),
                                    decoded.height(),
                                    image::ExtendedColorType::Rgb8,
                                )
                                .expect("could not encode image");
                            buf.flush().expect("could not flush buffer");

                            Some(buf.get_mut().clone())
                        }
                    }
                    None => None,
                };

                let result: Result<(i64,), sqlx::Error> =
                    sqlx::query_as(include_str!("../../queries/scan/create_album.sql"))
                        .bind(album)
                        .bind(metadata.sort_album.as_ref().unwrap_or(album))
                        .bind(artist_id)
                        .bind(resized_image)
                        .bind(thumb)
                        .bind(metadata.date)
                        .bind(&metadata.label)
                        .bind(&metadata.catalog)
                        .bind(&metadata.isrc)
                        .fetch_one(&self.pool)
                        .await;

                match result {
                    Ok(v) => Some(v.0),
                    Err(e) => {
                        error!("Database error while creating album: {:?}", e);
                        None
                    }
                }
            }
            Err(e) => {
                error!("Database error while retriving album: {:?}", e);
                None
            }
        }
    }

    async fn insert_track(
        &self,
        metadata: &Metadata,
        album_id: Option<i64>,
        path: &Path,
        length: u64,
    ) {
        // literally i do not know how this could possibly fail
        let name = metadata
            .name
            .clone()
            .or_else(|| {
                path.file_name()
                    .and_then(|x| x.to_str())
                    .map(|x| x.to_string())
            })
            .expect("weird file recieved in update metadata");

        let result: Result<(i64,), sqlx::Error> =
            sqlx::query_as(include_str!("../../queries/scan/create_track.sql"))
                .bind(&name)
                .bind(&name)
                .bind(album_id)
                .bind(metadata.track_current.map(|x| x as i32))
                .bind(metadata.disc_current.map(|x| x as i32))
                .bind(length as i32)
                .bind(path.to_str())
                .bind(&metadata.genre)
                .fetch_one(&self.pool)
                .await;

        match result {
            Ok(_) => (),
            Err(sqlx::Error::RowNotFound) => (),
            Err(e) => {
                error!("Database error while creating track: {:?}", e);
            }
        }
    }

    async fn update_metadata(
        &mut self,
        metadata: (Metadata, u64, Option<Box<[u8]>>),
        path: &Path,
    ) -> anyhow::Result<()> {
        debug!(
            "Adding/updating record for {:?} - {:?}",
            metadata.0.artist, metadata.0.name
        );

        let artist_id = self.insert_artist(&metadata.0).await;
        let album_id = self.insert_album(&metadata.0, artist_id, &metadata.2).await;
        self.insert_track(&metadata.0, album_id, path, metadata.1)
            .await;

        Ok(())
    }

    fn read_metadata_for_path(&mut self, path: &PathBuf) -> Option<FileInformation> {
        for (exts, provider) in &mut self.provider_table {
            if file_is_scannable_with_provider(path, exts) {
                if let Ok(mut metadata) = scan_file_with_provider(path, provider) {
                    if metadata.2.is_none() {
                        metadata.2 = scan_path_for_album_art(path);
                    }

                    return Some(metadata);
                }
            }
        }

        None
    }

    fn write_scan_record(&self) {
        if let Some(path) = self.scan_record_path.as_ref() {
            let mut file = File::create(path).unwrap();
            let data = serde_json::to_string(&self.scan_record).unwrap();
            if let Err(err) = file.write_all(data.as_bytes()) {
                error!("Could not write scan record: {:?}", err);
                error!("Scan record will not be saved, this may cause rescans on restart");
            } else {
                info!("Scan record written to {:?}", path);
            }
        } else {
            error!("No scan record path set, scan record will not be saved");
        }
    }

    fn scan(&mut self) {
        if self.to_process.is_empty() {
            info!("Scan complete, writing scan record and stopping");
            self.write_scan_record();
            self.scan_state = ScanState::Idle;
            self.event_tx.send(ScanEvent::ScanCompleteIdle).unwrap();
            return;
        }

        let path = self.to_process.pop().unwrap();
        let metadata = self.read_metadata_for_path(&path);

        if let Some(metadata) = metadata {
            task::block_on(self.update_metadata(metadata, &path)).unwrap();

            self.scanned += 1;

            if self.scanned % 5 == 0 {
                self.event_tx
                    .send(ScanEvent::ScanProgress {
                        current: self.scanned,
                        total: self.discovered_total,
                    })
                    .unwrap();
            }
        } else {
            warn!("Could not read metadata for file: {:?}", path);
        }
    }

    async fn delete_track(&mut self, path: &PathBuf) {
        debug!("track deleted or moved: {:?}", path);
        let result = sqlx::query(include_str!("../../queries/scan/delete_track.sql"))
            .bind(path.to_str())
            .execute(&self.pool)
            .await;

        if let Err(e) = result {
            error!("Database error while deleting track: {:?}", e);
        } else {
            self.scan_record.remove(path);
        }
    }

    // This is done in one shot because it's required for data integrity
    // Cleanup cannot be cancelled
    fn cleanup(&mut self) {
        self.scan_record
            .clone()
            .iter()
            .filter(|v| !v.0.exists())
            .map(|v| v.0)
            .for_each(|v| {
                task::block_on(self.delete_track(v));
            });

        self.scan_state = ScanState::Discovering;
    }
}

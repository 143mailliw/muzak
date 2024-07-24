use std::{
    hash::{Hash, Hasher},
    io::Cursor,
    sync::{
        mpsc::{Receiver, Sender},
        Arc,
    },
    thread::sleep,
};

use ahash::{AHashMap, RandomState};
use gpui::ImageData;
use image::imageops::thumbnail;
use tracing::{debug, warn};

use crate::{
    media::{builtin::symphonia::SymphoniaProvider, metadata::Metadata, traits::MediaProvider},
    util::rgb_to_bgr,
};

use super::{
    events::{DataCommand, DataEvent, ImageLayout, ImageType},
    interface::DataInterface,
    types::UIQueueItem,
};

fn create_generic_queue_item(path: String) -> UIQueueItem {
    UIQueueItem {
        metadata: Metadata {
            name: path
                .split(std::path::MAIN_SEPARATOR_STR)
                .last()
                .map(|v| v.to_string()),
            ..Default::default()
        },
        file_path: path,
        album_art: None,
    }
}

pub struct DataThread {
    commands_rx: Receiver<DataCommand>,
    events_tx: Sender<DataEvent>,
    image_cache: AHashMap<u64, Arc<ImageData>>,
    // TODO: get metadata from other providers as well
    media_provider: Box<dyn MediaProvider>,
    hash_state: RandomState,
}

impl DataThread {
    /// Starts the data thread and returns the created interface.
    pub fn start<T: DataInterface>() -> T {
        let (commands_tx, commands_rx) = std::sync::mpsc::channel();
        let (events_tx, events_rx) = std::sync::mpsc::channel();

        std::thread::Builder::new()
            .name("data".to_string())
            .spawn(move || {
                let mut thread = DataThread {
                    commands_rx,
                    events_tx,
                    image_cache: AHashMap::new(),
                    media_provider: Box::new(SymphoniaProvider::default()),
                    hash_state: RandomState::new(),
                };

                thread.run();
            })
            .expect("could not start data thread");

        T::new(commands_tx, events_rx)
    }

    fn run(&mut self) {
        while let Ok(command) = self.commands_rx.recv() {
            match command {
                DataCommand::DecodeImage(data, image_type, layout) => {
                    if self.decode_image(data, image_type, layout).is_err() {
                        self.events_tx
                            .send(DataEvent::DecodeError(image_type))
                            .expect("could not send event");
                    }
                }
                DataCommand::ReadQueueMetadata(paths) => {
                    let items = self.read_metadata_for_queue(paths);

                    self.events_tx
                        .send(DataEvent::MetadataRead(items))
                        .expect("could not send event");
                }
                DataCommand::EvictQueueCache => self.evict_unneeded_data(),
            }

            sleep(std::time::Duration::from_millis(10));
        }
    }

    // The only real possible error here is if the image format is unsupported, or the image is
    // corrupt. In either case, there's literally nothing we can do about it, and the only
    // required information is that there was an error. So, we just return `Result<(), ()>`.
    fn decode_image(
        &self,
        data: Box<[u8]>,
        image_type: ImageType,
        image_layout: ImageLayout,
    ) -> Result<(), ()> {
        let mut image = image::io::Reader::new(Cursor::new(data.clone()))
            .with_guessed_format()
            .map_err(|_| ())?
            .decode()
            .map_err(|_| ())?
            .into_rgba8();

        if image_layout == ImageLayout::BGR {
            rgb_to_bgr(&mut image);
        }

        self.events_tx
            .send(DataEvent::ImageDecoded(
                Arc::new(ImageData::new(thumbnail(&image, 80, 80))),
                image_type,
            ))
            .expect("could not send event");

        Ok(())
    }

    fn read_metadata_for_queue(&mut self, queue_items: Vec<String>) -> Vec<UIQueueItem> {
        let mut items = vec![];

        for path in queue_items {
            let file = if let Ok(file) = std::fs::File::open(path.clone()) {
                file
            } else {
                warn!("Failed to open file {}, queue may be desynced", path);
                warn!("Ensure the file exists before placing it in the queue");
                continue;
            };

            if self
                .media_provider
                .open(file, path.split(".").last().map(|v| v.to_string()))
                .is_err()
            {
                warn!("Media provider couldn't open file, creating generic queue item");
                items.push(create_generic_queue_item(path));
                continue;
            }

            if self.media_provider.start_playback().is_err() {
                warn!("Media provider couldn't start playback, creating generic queue item");
                items.push(create_generic_queue_item(path));
                continue;
            }

            let metadata = if let Ok(metadata) = self.media_provider.read_metadata() {
                metadata.clone()
            } else {
                warn!("Media provider couldn't retrieve metadata, creating generic queue item");
                items.push(create_generic_queue_item(path));
                continue;
            };

            let album_art = self
                .media_provider
                .read_image()
                .ok()
                .flatten()
                .map(|v| {
                    // we do this because we do not want to be storing entire encoded images
                    // long-term, collisions don't particuarly matter here so the benefits outweigh
                    // the tradeoffs
                    let key = self.hash_state.hash_one(v.clone());

                    if let Some(cached) = self.image_cache.get(&key) {
                        debug!("Image cache hit for key {}", key);
                        Some(cached.clone())
                    } else {
                        debug!("Image cache miss for key {}, decoding and caching", key);
                        let mut image = image::io::Reader::new(Cursor::new(v.clone()))
                            .with_guessed_format()
                            .map_err(|_| ())
                            .ok()?
                            .decode()
                            .ok()?
                            .into_rgba8();

                        rgb_to_bgr(&mut image);

                        let value = Arc::new(ImageData::new(thumbnail(&image, 80, 80)));
                        self.image_cache.insert(key, value.clone());

                        Some(value)
                    }
                })
                .flatten();

            items.push(UIQueueItem {
                metadata,
                file_path: path,
                album_art,
            })
        }

        items
    }

    fn evict_unneeded_data(&mut self) {
        // we have to duplicate this data in order to get around borrowing rules
        let keys: Vec<u64> = self.image_cache.keys().cloned().collect();

        for key in keys {
            let value = self.image_cache.get(&key).clone().unwrap();

            // no clue how this could possibly be less than 2 but it doesn't hurt to check
            if Arc::<gpui::ImageData>::strong_count(&value) <= 2 {
                debug!("evicting {}", key);
                self.image_cache.remove(&key);
            }
        }
    }
}

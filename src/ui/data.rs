use std::{fs::File, io::Cursor, sync::Arc};

use gpui::{App, AppContext, Entity, RenderImage, SharedString, Task};
use image::{imageops::thumbnail, Frame, ImageReader};
use smallvec::smallvec;
use tracing::error;

use crate::{
    media::{builtin::symphonia::SymphoniaProvider, traits::MediaProvider},
    playback::queue::{DataSource, QueueItemUIData},
    util::rgb_to_bgr,
};

fn decode_image(data: Box<[u8]>, thumb: bool) -> anyhow::Result<Arc<RenderImage>> {
    let mut image = ImageReader::new(Cursor::new(data))
        .with_guessed_format()?
        .decode()?
        .into_rgba8();

    rgb_to_bgr(&mut image);

    let frame = if thumb {
        Frame::new(thumbnail(&image, 80, 80))
    } else {
        Frame::new(image)
    };

    Ok(Arc::new(RenderImage::new(smallvec![frame])))
}

fn read_metadata(path: String) -> anyhow::Result<QueueItemUIData> {
    let file = File::open(&path)?;

    // TODO: Switch to a different media provider based on the file
    let mut media_provider = SymphoniaProvider::default();
    media_provider.open(file, None)?;
    media_provider.start_playback()?;
    let metadata = media_provider.read_metadata()?;

    // TODO: handle album art
    // needs a cache
    let album_art = None;

    Ok(QueueItemUIData {
        image: album_art,
        name: metadata.name.as_ref().map(SharedString::from),
        artist_name: metadata.artist.as_ref().map(SharedString::from),
        source: DataSource::Metadata,
    })
}

pub trait Decode {
    fn decode_image(
        &self,
        data: Box<[u8]>,
        thumb: bool,
        entity: Entity<Option<Arc<RenderImage>>>,
    ) -> Task<()>;
    fn read_metadata(&self, path: String, entity: Entity<Option<QueueItemUIData>>) -> Task<()>;
}

impl Decode for App {
    fn decode_image(
        &self,
        data: Box<[u8]>,
        thumb: bool,
        entity: Entity<Option<Arc<RenderImage>>>,
    ) -> Task<()> {
        self.spawn(|mut cx| async move {
            let decode_task = cx
                .background_spawn(async move { decode_image(data, thumb) })
                .await;

            let Ok(image) = decode_task else {
                error!("Failed to decode image - {:?}", decode_task);
                return;
            };

            entity
                .update(&mut cx, |m, cx| {
                    *m = Some(image);
                    cx.notify();
                })
                .expect("Failed to update entity");
        })
    }

    fn read_metadata(&self, path: String, entity: Entity<Option<QueueItemUIData>>) -> Task<()> {
        self.spawn(|mut cx| async move {
            let read_task = cx
                .background_spawn(async move { read_metadata(path) })
                .await;

            let Ok(metadata) = read_task else {
                error!("Failed to read metadata - {:?}", read_task);
                return;
            };

            entity
                .update(&mut cx, |m, cx| {
                    *m = Some(metadata);
                    cx.notify();
                })
                .expect("Failed to update entity");
        })
    }
}

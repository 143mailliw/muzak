use std::{fmt::Display, sync::Arc};

use gpui::{App, AppContext, Entity, RenderImage, SharedString};

use crate::{library::db::LibraryAccess, ui::data::Decode, ui::models::Models};

#[derive(Clone, Debug, PartialEq)]
pub struct QueueItemData {
    /// The UI data associated with the queue item.
    data: Entity<Option<QueueItemUIData>>,
    /// The database ID of track the item is from, if it exists.
    db_id: Option<i64>,
    /// The database ID of album the item is from, if it exists.
    db_album_id: Option<i64>,
    /// The path to the track file.
    path: String,
}

impl Display for QueueItemData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.path)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct QueueItemUIData {
    /// The image associated with the track, if it exists.
    pub image: Option<Arc<RenderImage>>,
    /// The name of the track, if it is known.
    pub name: Option<SharedString>,
    /// The name of the artist, if it is known.
    pub artist_name: Option<SharedString>,
    /// Whether the track's metadata is known from the file or the database.
    pub source: DataSource,
}

#[derive(Clone, Debug, PartialEq, Copy)]
pub enum DataSource {
    /// The metadata was read directly from the file.
    Metadata,
    /// The metadata was read from the library database.
    Library,
}

impl QueueItemData {
    /// Creates a new `QueueItemData` instance with the given information.
    pub fn new(cx: &mut App, path: String, db_id: Option<i64>, db_album_id: Option<i64>) -> Self {
        QueueItemData {
            path,
            db_id,
            db_album_id,
            data: cx.new(|_| None),
        }
    }

    /// Returns a copy of the UI data after ensuring that the metadata is loaded (or going to be
    /// loaded).
    pub fn get_data(&self, cx: &mut App) -> Entity<Option<QueueItemUIData>> {
        let model = self.data.clone();
        let track_id = self.db_id;
        let album_id = self.db_album_id;
        let path = self.path.clone();
        model.update(cx, move |m, cx| {
            // if we already have the data, exit the function
            if m.is_some() {
                return;
            }
            *m = Some(QueueItemUIData {
                image: None,
                name: None,
                artist_name: None,
                source: DataSource::Library,
            });

            // if the database ids are known we can get the data from the database
            if let (Some(track_id), Some(album_id)) = (track_id, album_id) {
                let album =
                    cx.get_album_by_id(album_id, crate::library::db::AlbumMethod::Thumbnail);
                let track = cx.get_track_by_id(track_id);

                if let (Ok(track), Ok(album)) = (track, album) {
                    m.as_mut().unwrap().name = Some(track.title.clone().into());
                    m.as_mut().unwrap().image = album.thumb.clone().map(|v| v.0);

                    if let Ok(artist) = cx.get_artist_by_id(album.artist_id) {
                        m.as_mut().unwrap().artist_name = artist.name.clone().map(|v| v.into());
                    }
                }

                cx.notify();
            }

            if m.as_ref().unwrap().artist_name.is_some() {
                return;
            }

            // vital information left blank, try retriving the metadata from disk
            // much slower, especially on windows
            let path_clone = path.clone();
            cx.read_metadata(path_clone, cx.entity()).detach();
        });

        model
    }

    /// Drop the UI data from the queue item. This means the data must be retrieved again from disk
    /// if the item is used with get_data again.
    pub fn drop_data(&self, cx: &mut App) {
        self.data.update(cx, |m, cx| {
            *m = None;
            cx.notify();
        });
    }

    /// Returns the file path of the queue item.
    pub fn get_path(&self) -> &String {
        &self.path
    }
}

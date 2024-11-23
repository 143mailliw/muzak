use std::sync::Arc;

use gpui::*;
use prelude::FluentBuilder;
use tracing::debug;

use crate::{
    data::{
        events::{ImageLayout, ImageType},
        interface::GPUIDataInterface,
    },
    library::{
        db::{AlbumMethod, LibraryAccess},
        types::{Album, Artist, Track},
    },
    playback::interface::{replace_queue, GPUIPlaybackInterface},
    ui::{
        app::DropOnNavigateQueue,
        components::{
            button::{button, ButtonIntent, ButtonSize},
            context::context,
            menu::{menu, menu_item},
        },
        constants::FONT_AWESOME,
        models::{Models, PlaybackInfo},
        theme::Theme,
    },
};

pub struct ReleaseView {
    album: Arc<Album>,
    image: Option<Arc<RenderImage>>,
    artist: Option<Arc<Artist>>,
    tracks: Arc<Vec<Track>>,
    track_list_state: ListState,
    release_info: Option<SharedString>,
}

impl ReleaseView {
    pub(super) fn new<V: 'static>(cx: &mut ViewContext<V>, album_id: i64) -> View<Self> {
        cx.new_view(|cx| {
            let image = None;
            // TODO: error handling
            let album = cx
                .get_album_by_id(album_id, AlbumMethod::Cached)
                .expect("Failed to retrieve album");
            let tracks = cx
                .list_tracks_in_album(album_id)
                .expect("Failed to retrieve tracks");
            let artist = cx.get_artist_by_id(album.artist_id).ok();

            let image_transfer_model = cx.global::<Models>().image_transfer_model.clone();

            cx.subscribe(
                &image_transfer_model,
                move |this: &mut ReleaseView, _, image, cx| {
                    if image.0 == ImageType::AlbumArt(album_id) {
                        debug!("captured decoded image for album ID: {}", album_id);
                        this.image = Some(image.1.clone());

                        cx.global::<DropOnNavigateQueue>().add(image.1.clone());
                        cx.notify();
                    }
                },
            )
            .detach();

            if let Some(image) = album.image.clone() {
                cx.global::<GPUIDataInterface>().decode_image(
                    image,
                    ImageType::AlbumArt(album_id),
                    ImageLayout::BGR,
                    false,
                );
            }

            let tracks_clone = tracks.clone();

            let state =
                ListState::new(tracks.len(), ListAlignment::Top, px(25.0), move |idx, _| {
                    TrackItem {
                        track: tracks_clone[idx].clone(),
                        is_start: if idx > 0 {
                            if let Some(track) = tracks_clone.get(idx - 1) {
                                track.disc_number != tracks_clone[idx].disc_number
                            } else {
                                true
                            }
                        } else {
                            true
                        },
                        tracks: tracks_clone.clone(),
                    }
                    .into_any_element()
                });

            let release_info = {
                let mut info = String::default();

                if let Some(label) = &album.label {
                    info += &label.to_string();
                }

                if album.label.is_some() && album.catalog_number.is_some() {
                    info += " • ";
                }

                if let Some(catalog_number) = &album.catalog_number {
                    info += &catalog_number.to_string();
                }

                if !info.is_empty() {
                    Some(SharedString::from(info))
                } else {
                    None
                }
            };

            ReleaseView {
                album,
                image,
                artist,
                tracks,
                track_list_state: state,
                release_info,
            }
        })
    }
}

impl Render for ReleaseView {
    fn render(&mut self, cx: &mut ViewContext<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();

        div()
            .mt(px(24.0))
            .w_full()
            .flex_shrink()
            .overflow_x_hidden()
            .h_full()
            .max_w(px(1000.0))
            .mx_auto()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_shrink()
                    .flex()
                    .overflow_x_hidden()
                    .px(px(24.0))
                    .w_full()
                    .child(
                        div()
                            .rounded(px(4.0))
                            .bg(theme.album_art_background)
                            .shadow_sm()
                            .w(px(160.0))
                            .h(px(160.0))
                            .flex_shrink_0()
                            .overflow_hidden()
                            .when(self.image.is_some(), |div| {
                                div.child(
                                    img(self.image.clone().unwrap())
                                        .min_w(px(160.0))
                                        .min_h(px(160.0))
                                        .max_w(px(160.0))
                                        .max_h(px(160.0))
                                        .overflow_hidden()
                                        .flex()
                                        // TODO: Ideally this should be ObjectFit::Cover, but for
                                        // some reason that makes the element bigger
                                        // FIXME: Is this a GPUI bug?
                                        .object_fit(ObjectFit::Fill)
                                        .rounded(px(4.0)),
                                )
                            }),
                    )
                    .child(
                        div()
                            .ml(px(18.0))
                            .mt_auto()
                            .flex_shrink()
                            .flex()
                            .flex_col()
                            .w_full()
                            .overflow_x_hidden()
                            .child(div().font_weight(FontWeight::SEMIBOLD).when_some(
                                self.artist.as_ref().map(|v| v.name.clone()),
                                |this, artist| this.child(artist.unwrap()),
                            ))
                            .child(
                                div()
                                    .font_weight(FontWeight::EXTRA_BOLD)
                                    .text_size(rems(2.5))
                                    .line_height(rems(2.75))
                                    .overflow_x_hidden()
                                    .pb(px(10.0))
                                    .min_w_0()
                                    .text_ellipsis()
                                    .child(self.album.title.clone()),
                            )
                            .child(
                                div()
                                    .gap(px(10.0))
                                    .flex()
                                    .flex_row()
                                    .child(
                                        button()
                                            .id("release-play-button")
                                            .size(ButtonSize::Large)
                                            .font_weight(FontWeight::BOLD)
                                            .intent(ButtonIntent::Primary)
                                            .on_click(cx.listener(
                                                |this: &mut ReleaseView, _, cx| {
                                                    let paths = this
                                                        .tracks
                                                        .iter()
                                                        .map(|track| track.location.clone())
                                                        .collect();

                                                    replace_queue(paths, cx)
                                                },
                                            ))
                                            .child(div().font_family(FONT_AWESOME).child(""))
                                            .child(div().child("Play")),
                                    )
                                    .child(
                                        button()
                                            .id("release-add-button")
                                            .size(ButtonSize::Large)
                                            .font_weight(FontWeight::BOLD)
                                            .flex_none()
                                            .on_click(cx.listener(
                                                |this: &mut ReleaseView, _, cx| {
                                                    let paths = this
                                                        .tracks
                                                        .iter()
                                                        .map(|track| track.location.clone())
                                                        .collect();

                                                    cx.global::<GPUIPlaybackInterface>()
                                                        .queue_list(paths);
                                                },
                                            ))
                                            .child(div().font_family(FONT_AWESOME).child("")),
                                    )
                                    .child(
                                        button()
                                            .id("release-shuffle-button")
                                            .size(ButtonSize::Large)
                                            .font_weight(FontWeight::BOLD)
                                            .flex_none()
                                            .on_click(cx.listener(
                                                |this: &mut ReleaseView, _, cx| {
                                                    let paths = this
                                                        .tracks
                                                        .iter()
                                                        .map(|track| track.location.clone())
                                                        .collect();

                                                    if !(*cx
                                                        .global::<PlaybackInfo>()
                                                        .shuffling
                                                        .read(cx))
                                                    {
                                                        cx.global::<GPUIPlaybackInterface>()
                                                            .toggle_shuffle();
                                                    }

                                                    replace_queue(paths, cx)
                                                },
                                            ))
                                            .child(div().font_family(FONT_AWESOME).child("")),
                                    ),
                            ),
                    ),
            )
            .child(
                list(self.track_list_state.clone())
                    .w_full()
                    .flex()
                    .h_full()
                    .flex_col()
                    .mx_auto(),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .text_sm()
                    .ml(px(24.0))
                    .pt(px(12.0))
                    .pb(px(24.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text_secondary)
                    .when_some(self.release_info.clone(), |this, release_info| {
                        this.child(div().child(release_info))
                    })
                    .when_some(self.album.release_date, |this, date| {
                        this.child(div().child(format!("Released {}", date.format("%B %-e, %Y"))))
                    })
                    .when_some(self.album.isrc.as_ref(), |this, isrc| {
                        this.child(div().child(isrc.clone()))
                    }),
            )
    }
}

#[derive(IntoElement)]
struct TrackItem {
    pub track: Track,
    pub is_start: bool,
    pub tracks: Arc<Vec<Track>>,
}

impl RenderOnce for TrackItem {
    fn render(self, cx: &mut WindowContext) -> impl IntoElement {
        let theme = cx.global::<Theme>();

        let tracks = self.tracks.clone();
        let tracks_2 = self.tracks.clone();
        let track_location = self.track.location.clone();
        let track_location_2 = self.track.location;
        let track_id = self.track.id;
        context(("context", self.track.id as usize))
            .with(
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .id(self.track.id as usize)
                    .on_click(move |_, cx| play_from_track(cx, &tracks, track_id))
                    .when(self.is_start, |this| {
                        this.child(
                            div()
                                .text_color(theme.text_secondary)
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .px(px(24.0))
                                .border_b_1()
                                .w_full()
                                .border_color(theme.border_color)
                                .mt(px(24.0))
                                .pb(px(6.0))
                                .child(format!(
                                    "DISC {}",
                                    self.track.disc_number.unwrap_or_default()
                                )),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .border_b_1()
                            .id(("track", self.track.id as u64))
                            .w_full()
                            .border_color(theme.border_color)
                            .cursor_pointer()
                            .px(px(24.0))
                            .py(px(6.0))
                            .hover(|this| this.bg(theme.nav_button_hover))
                            .active(|this| this.bg(theme.nav_button_active))
                            .max_w_full()
                            .child(
                                div()
                                    .w(px(62.0))
                                    .font_family("Roboto Mono")
                                    .flex_shrink_0()
                                    .child(format!(
                                        "{}",
                                        self.track.track_number.unwrap_or_default()
                                    )),
                            )
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .overflow_x_hidden()
                                    .text_ellipsis()
                                    .child(self.track.title),
                            )
                            .child(
                                div()
                                    .font_family("Roboto Mono")
                                    .ml_auto()
                                    .flex_shrink_0()
                                    .child(format!(
                                        "{}:{:02}",
                                        self.track.duration / 60,
                                        self.track.duration % 60
                                    )),
                            ),
                    ),
            )
            .child(
                div().bg(theme.elevated_background).child(
                    menu()
                        .item(menu_item(
                            "track_play",
                            Some(""),
                            "Play",
                            move |_, cx| {
                                let playback_interface = cx.global::<GPUIPlaybackInterface>();
                                let queue_length = cx.global::<Models>().queue.read(cx).0.len();
                                playback_interface.queue(&track_location);
                                playback_interface.jump(queue_length);
                            },
                        ))
                        .item(menu_item(
                            "track_play_from_here",
                            Some(""),
                            "Play from here",
                            move |_, cx| play_from_track(cx, &tracks_2, track_id),
                        ))
                        .item(menu_item(
                            "track_add_to_queue",
                            Some("+"),
                            "Add to queue",
                            move |_, cx| {
                                let playback_interface = cx.global::<GPUIPlaybackInterface>();
                                playback_interface.queue(&track_location_2);
                            },
                        )),
                ),
            )
    }
}

fn play_from_track(cx: &mut WindowContext, tracks: &Arc<Vec<Track>>, id: i64) {
    let paths = tracks.iter().map(|track| track.location.clone()).collect();

    replace_queue(paths, cx);

    let playback_interface = cx.global::<GPUIPlaybackInterface>();
    playback_interface.jump(tracks.iter().position(|t| t.id == id).unwrap())
}

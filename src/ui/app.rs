use core::panic;
use std::{
    fs,
    sync::{Arc, RwLock},
};

use directories::ProjectDirs;
use gpui::*;
use prelude::FluentBuilder;
use sqlx::SqlitePool;
use tracing::{debug, error};

use crate::{
    library::{
        db::create_pool,
        scan::{ScanInterface, ScanThread},
    },
    playback::{interface::GPUIPlaybackInterface, queue::QueueItemData, thread::PlaybackThread},
    settings::{setup_settings, SettingsGlobal},
};

use super::{
    arguments::parse_args_and_prepare,
    assets::Assets,
    components::{input, modal},
    constants::APP_ROUNDING,
    controls::Controls,
    global_actions::register_actions,
    header::Header,
    library::Library,
    models::{self, build_models},
    queue::Queue,
    search::SearchView,
    theme::{setup_theme, Theme},
    util::drop_image_from_app,
};

struct WindowShadow {
    pub controls: Entity<Controls>,
    pub queue: Entity<Queue>,
    pub library: Entity<Library>,
    pub header: Entity<Header>,
    pub search: Entity<SearchView>,
    pub show_queue: Entity<bool>,
}

impl Render for WindowShadow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();

        let decorations = window.window_decorations();
        let rounding = APP_ROUNDING;
        let shadow_size = px(10.0);
        let border_size = px(1.0);
        window.set_client_inset(shadow_size);

        let queue = self.queue.clone();

        div()
            .id("window-backdrop")
            .key_context("app")
            .bg(transparent_black())
            .map(|div| match decorations {
                Decorations::Server => div,
                Decorations::Client { tiling, .. } => div
                    .bg(gpui::transparent_black())
                    .child(
                        canvas(
                            |_bounds, window, _| {
                                window.insert_hitbox(
                                    Bounds::new(
                                        point(px(0.0), px(0.0)),
                                        window.window_bounds().get_bounds().size,
                                    ),
                                    false,
                                )
                            },
                            move |_bounds, hitbox, window, _| {
                                let mouse = window.mouse_position();
                                let size = window.window_bounds().get_bounds().size;
                                let Some(edge) = resize_edge(mouse, shadow_size, size, tiling)
                                else {
                                    return;
                                };
                                window.set_cursor_style(
                                    match edge {
                                        ResizeEdge::Top | ResizeEdge::Bottom => {
                                            CursorStyle::ResizeUpDown
                                        }
                                        ResizeEdge::Left | ResizeEdge::Right => {
                                            CursorStyle::ResizeLeftRight
                                        }
                                        ResizeEdge::TopLeft | ResizeEdge::BottomRight => {
                                            CursorStyle::ResizeUpLeftDownRight
                                        }
                                        ResizeEdge::TopRight | ResizeEdge::BottomLeft => {
                                            CursorStyle::ResizeUpRightDownLeft
                                        }
                                    },
                                    &hitbox,
                                );
                            },
                        )
                        .size_full()
                        .absolute(),
                    )
                    .when(!(tiling.top || tiling.right), |div| {
                        div.rounded_tr(rounding)
                    })
                    .when(!(tiling.top || tiling.left), |div| div.rounded_tl(rounding))
                    .when(!(tiling.bottom || tiling.right), |div| {
                        div.rounded_br(rounding)
                    })
                    .when(!(tiling.bottom || tiling.left), |div| {
                        div.rounded_bl(rounding)
                    })
                    .when(!tiling.top, |div| div.pt(shadow_size))
                    .when(!tiling.bottom, |div| div.pb(shadow_size))
                    .when(!tiling.left, |div| div.pl(shadow_size))
                    .when(!tiling.right, |div| div.pr(shadow_size))
                    .on_mouse_down(MouseButton::Left, move |e, window, _| {
                        let size = window.window_bounds().get_bounds().size;
                        let pos = e.position;

                        if let Some(edge) = resize_edge(pos, shadow_size, size, tiling) {
                            window.start_window_resize(edge)
                        };
                    }),
            })
            .size_full()
            .child(
                div()
                    .font_family("Inter")
                    .text_color(theme.text)
                    .cursor(CursorStyle::Arrow)
                    .map(|div| match decorations {
                        Decorations::Server => div,
                        Decorations::Client { tiling } => div
                            .when(cfg!(not(target_os = "macos")), |div| {
                                div.border_color(rgba(0x64748b33))
                            })
                            .when(!(tiling.top || tiling.right), |div| {
                                div.rounded_tr(rounding)
                            })
                            .when(!(tiling.top || tiling.left), |div| div.rounded_tl(rounding))
                            .when(!(tiling.bottom || tiling.right), |div| {
                                div.rounded_br(rounding)
                            })
                            .when(!(tiling.bottom || tiling.left), |div| {
                                div.rounded_bl(rounding)
                            })
                            .when(!tiling.top, |div| div.border_t(border_size))
                            .when(!tiling.bottom, |div| div.border_b(border_size))
                            .when(!tiling.left, |div| div.border_l(border_size))
                            .when(!tiling.right, |div| div.border_r(border_size))
                            .when(!tiling.is_tiled(), |div| {
                                div.shadow(smallvec::smallvec![gpui::BoxShadow {
                                    color: Hsla {
                                        h: 0.,
                                        s: 0.,
                                        l: 0.,
                                        a: 0.4,
                                    },
                                    blur_radius: shadow_size / 2.,
                                    spread_radius: px(0.),
                                    offset: point(px(0.0), px(0.0)),
                                }])
                            }),
                    })
                    .on_mouse_move(|_e, _, cx| {
                        cx.stop_propagation();
                    })
                    .overflow_hidden()
                    .bg(theme.background_primary)
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(self.header.clone())
                    .child(
                        div()
                            .w_full()
                            .h_full()
                            .flex()
                            .relative()
                            .max_w_full()
                            .max_h_full()
                            .overflow_hidden()
                            .child(self.library.clone())
                            .when(*self.show_queue.read(cx), |this| this.child(queue)),
                    )
                    .child(self.controls.clone())
                    .child(self.search.clone()),
            )
    }
}

fn resize_edge(
    pos: Point<Pixels>,
    shadow_size: Pixels,
    size: Size<Pixels>,
    tiling: Tiling,
) -> Option<ResizeEdge> {
    let edge = if pos.y < shadow_size && pos.x < shadow_size && !tiling.top && !tiling.left {
        ResizeEdge::TopLeft
    } else if pos.y < shadow_size
        && pos.x > size.width - shadow_size
        && !tiling.top
        && !tiling.right
    {
        ResizeEdge::TopRight
    } else if pos.y < shadow_size && !tiling.top {
        ResizeEdge::Top
    } else if pos.y > size.height - shadow_size
        && pos.x < shadow_size
        && !tiling.bottom
        && !tiling.left
    {
        ResizeEdge::BottomLeft
    } else if pos.y > size.height - shadow_size
        && pos.x > size.width - shadow_size
        && !tiling.bottom
        && !tiling.right
    {
        ResizeEdge::BottomRight
    } else if pos.y > size.height - shadow_size && !tiling.bottom {
        ResizeEdge::Bottom
    } else if pos.x < shadow_size && !tiling.left {
        ResizeEdge::Left
    } else if pos.x > size.width - shadow_size && !tiling.right {
        ResizeEdge::Right
    } else {
        return None;
    };
    Some(edge)
}

pub fn find_fonts(cx: &mut App) -> gpui::Result<()> {
    let paths = cx.asset_source().list("fonts")?;
    let mut fonts = vec![];
    for path in paths {
        if path.ends_with(".ttf") || path.ends_with(".otf") {
            if let Some(v) = cx.asset_source().load(&path)? {
                fonts.push(v);
            }
        }
    }

    let results = cx.text_system().add_fonts(fonts);
    debug!("loaded fonts: {:?}", cx.text_system().all_font_names());
    results
}

pub struct Pool(pub SqlitePool);

impl Global for Pool {}

pub fn get_dirs() -> ProjectDirs {
    directories::ProjectDirs::from("me", "william341", "muzak").expect("couldn't find project dirs")
}

pub struct DropImageDummyModel;

impl EventEmitter<Vec<Arc<RenderImage>>> for DropImageDummyModel {}

pub async fn run() {
    let dirs = get_dirs();
    let directory = dirs.data_dir().to_path_buf();
    if !directory.exists() {
        fs::create_dir_all(&directory)
            .unwrap_or_else(|e| panic!("couldn't create data directory, {:?}, {:?}", directory, e));
    }
    let file = directory.join("library.db");

    let pool = create_pool(file).await;

    Application::new()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            let bounds = Bounds::centered(None, size(px(1024.0), px(700.0)), cx);
            find_fonts(cx).expect("unable to load fonts");

            register_actions(cx);

            let queue: Arc<RwLock<Vec<QueueItemData>>> = Arc::new(RwLock::new(Vec::new()));

            build_models(
                cx,
                models::Queue {
                    data: queue.clone(),
                    position: 0,
                },
            );

            input::bind_actions(cx);
            modal::bind_actions(cx);

            setup_theme(cx, directory.join("theme.json"));
            setup_settings(cx, directory.join("settings.json"));

            if let Ok(pool) = pool {
                let settings = cx.global::<SettingsGlobal>().model.read(cx);
                let mut scan_interface: ScanInterface =
                    ScanThread::start(pool.clone(), settings.scanning.clone());
                scan_interface.scan();
                scan_interface.start_broadcast(cx);

                cx.set_global(scan_interface);
                cx.set_global(Pool(pool));
            } else {
                error!("unable to create database pool: {}", pool.err().unwrap());
                panic!("fatal: unable to create database pool");
            }

            let drop_model = cx.new(|_| DropImageDummyModel);

            cx.subscribe(&drop_model, |_, vec, cx| {
                for image in vec.clone() {
                    drop_image_from_app(cx, image);
                }
            })
            .detach();

            let mut playback_interface: GPUIPlaybackInterface = PlaybackThread::start(queue);
            playback_interface.start_broadcast(cx);
            parse_args_and_prepare(cx, &playback_interface);
            cx.set_global(playback_interface);

            cx.activate(true);

            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_background: WindowBackgroundAppearance::Opaque,
                    window_decorations: Some(WindowDecorations::Client),
                    window_min_size: Some(size(px(800.0), px(600.0))),
                    titlebar: Some(TitlebarOptions {
                        title: Some(SharedString::from("Muzak")),
                        // TODO: fix this
                        appears_transparent: true,
                        traffic_light_position: Some(Point {
                            x: px(9.0),
                            y: px(9.0),
                        }),
                    }),
                    kind: WindowKind::Normal,
                    ..Default::default()
                },
                |window, cx| {
                    cx.new(|cx| {
                        cx.observe_window_appearance(window, |_, _, cx| {
                            cx.refresh_windows();
                        })
                        .detach();

                        let show_queue = cx.new(|_| true);

                        WindowShadow {
                            controls: Controls::new(cx, show_queue.clone()),
                            queue: Queue::new(cx, show_queue.clone()),
                            library: Library::new(cx),
                            header: Header::new(cx),
                            search: SearchView::new(cx),
                            show_queue,
                        }
                    })
                },
            )
            .unwrap();
        });
}

use super::*;

#[test]
fn a_skin_packs_band_goes_on_the_side_facing_the_screen_centre() {
    use crate::look::skins::tests::{decode_png, pack, rows_png};
    let skins = tempfile::tempdir().expect("temp dir");
    let folder = pack(
        skins.path(),
        "stripes",
        r#"{ "image": "t.png", "shadowBand": 2 }"#,
    );
    rows_png(
        &folder.join("t.png"),
        3,
        &[[10, 20].as_slice(), &[100; 25]].concat(),
    );
    let mut settings = Settings {
        theme: crate::config::model::Theme::Classic,
        ..Settings::default()
    };
    settings.classic.skin = "stripes".into();
    let (mut logic, _window, _host, _config) = bar(settings);
    logic.skin = SkinCache::with_decoder(skins.path().into(), decode_png);
    let size = SizeF::new(WIDTH, 25.0);
    logic.paint(&mut RecordingCanvas::new(1.0), size);
    // At the bottom of the screen the band goes above the bar, as wide as the bar.
    let band = logic.band().expect("a band");
    assert!(band.above);
    assert_eq!((band.bitmap.width(), band.bitmap.height()), (1200, 2));
    // The next frame hands over the same band, which the window need not draw again.
    logic.paint(&mut RecordingCanvas::new(1.0), size);
    assert!(logic.band().expect("a band").same(&band));
    // Placed at the top: below it.
    if let Some(g) = logic.geometry.as_mut() {
        g.edge = Edge::Top;
    }
    logic.paint(&mut RecordingCanvas::new(1.0), size);
    assert!(!logic.band().expect("a band").above);
    // Built-in skins and the modern bar have none.
    logic.theme.skin = None;
    logic.paint(&mut RecordingCanvas::new(1.0), size);
    assert_eq!(logic.band(), None);
}

#[test]
fn a_pack_installed_later_is_drawn_once_the_bar_hears_of_it() {
    use crate::look::skins::tests::{decode_png, pack, rows_png};
    let skins = tempfile::tempdir().expect("temp dir");
    let mut settings = Settings {
        theme: crate::config::model::Theme::Classic,
        ..Settings::default()
    };
    settings.classic.skin = "stripes".into();
    let (mut logic, window, _host, _config) = bar(settings);
    logic.skin = SkinCache::with_decoder(skins.path().into(), decode_png);
    let size = SizeF::new(WIDTH, 25.0);
    // Not installed: Gloss, without a band.
    logic.paint(&mut RecordingCanvas::new(1.0), size);
    assert_eq!(logic.band(), None);
    let folder = pack(
        skins.path(),
        "stripes",
        r#"{ "image": "t.png", "shadowBand": 2 }"#,
    );
    rows_png(
        &folder.join("t.png"),
        3,
        &[[10, 20].as_slice(), &[100; 25]].concat(),
    );
    // The skin drawn is kept until the bar hears of it.
    logic.paint(&mut RecordingCanvas::new(1.0), size);
    assert_eq!(logic.band(), None);
    // Settings installed it and tells the bar (SPEC §12.4).
    let (commands, queue) = mpsc::channel();
    logic.commands = queue;
    commands.send(Command::SkinsChanged).expect("queued");
    logic.on_event(&window, UiEvent::Wake);
    logic.paint(&mut RecordingCanvas::new(1.0), size);
    assert!(logic.band().is_some());
}

#[test]
fn the_bars_shadow_goes_where_a_skins_band_would() {
    use crate::look::skins::tests::{decode_png, pack, rows_png};
    let mut settings = Settings::default();
    settings.bar.shadow = true;
    // None while the bar rises and its content fades in (animations on, whatever the OS
    // says: CI's Windows runner has them off).
    let mut animated = settings.clone();
    animated.bar.animations = crate::config::model::Animations::On;
    let (mut rising, ..) = rising_bar(animated);
    rising.paint(&mut RecordingCanvas::new(1.0), SizeF::new(WIDTH, 28.0));
    assert_eq!(rising.band(), None);
    let (mut logic, window, _host, config) = bar(settings);
    let size = SizeF::new(WIDTH, 28.0);
    // The modern bar at the bottom of the screen: 6 px of shadow above it, as wide as it.
    logic.paint(&mut RecordingCanvas::new(1.0), size);
    let band = logic.band().expect("a shadow");
    assert!(band.above);
    assert_eq!((band.bitmap.width(), band.bitmap.height()), (1200, 6));
    // The next frame hands over the same shadow; at 150 % it is wider and 9 px deep.
    logic.paint(&mut RecordingCanvas::new(1.0), size);
    assert!(logic.band().expect("a shadow").same(&band));
    logic.paint(&mut RecordingCanvas::new(1.5), size);
    let scaled = logic.band().expect("a shadow").bitmap;
    assert_eq!((scaled.width(), scaled.height()), (1800, 9));
    // Placed at the top (or floating nearer the top): below it.
    if let Some(g) = logic.geometry.as_mut() {
        g.edge = Edge::Top;
    }
    logic.paint(&mut RecordingCanvas::new(1.0), size);
    assert!(!logic.band().expect("a shadow").above);

    // A skin pack's own band takes its place (SPEC §12.4).
    let skins = tempfile::tempdir().expect("temp dir");
    let folder = pack(
        skins.path(),
        "stripes",
        r#"{ "image": "t.png", "shadowBand": 2 }"#,
    );
    rows_png(
        &folder.join("t.png"),
        3,
        &[[10, 20].as_slice(), &[100; 25]].concat(),
    );
    let mut classic = config.clone();
    classic.settings.theme = crate::config::model::Theme::Classic;
    classic.settings.classic.skin = "stripes".into();
    logic.skin = SkinCache::with_decoder(skins.path().into(), decode_png);
    logic.apply_config(&window, classic.clone());
    logic.refresh_theme(&window);
    let size = SizeF::new(WIDTH, 25.0);
    logic.paint(&mut RecordingCanvas::new(1.0), size);
    assert_eq!(logic.band().expect("the pack's band").bitmap.height(), 2);
    // A built-in skin has none: the shadow again.
    classic.settings.classic.skin = "gloss".into();
    logic.apply_config(&window, classic.clone());
    logic.refresh_theme(&window);
    logic.paint(&mut RecordingCanvas::new(1.0), size);
    assert_eq!(logic.band().expect("a shadow").bitmap.height(), 6);
    // Turned off: nothing beside the bar.
    classic.settings.bar.shadow = false;
    logic.apply_config(&window, classic);
    logic.paint(&mut RecordingCanvas::new(1.0), size);
    assert_eq!(logic.band(), None);
}

#[test]
fn highlight_fades_in_while_the_pointer_is_over_the_bar() {
    let mut settings = Settings::default();
    settings.bar.highlight_on_hover = true;
    let (mut logic, window, _, _) = bar(settings);
    let empty = PointF::new(300.0, 14.0);
    logic.on_event(&window, UiEvent::PointerMove(empty));
    assert_eq!(logic.highlight.target(), 1.0);
    assert_eq!(window.frames.get(), 1);
    // Moving within the bar starts nothing new.
    logic.on_event(&window, UiEvent::PointerMove(LOGO));
    assert_eq!(window.frames.get(), 1);
    logic.on_event(&window, UiEvent::PointerLeave);
    assert_eq!(logic.highlight.target(), 0.0);
    assert_eq!(window.frames.get(), 2);

    // Off by default.
    let (mut logic, window, _, _) = bar(Settings::default());
    logic.on_event(&window, UiEvent::PointerMove(empty));
    assert_eq!(logic.highlight.target(), 0.0);
    assert_eq!(window.frames.get(), 0);
}

#[test]
fn rises_into_place_and_slides_out_before_changing_edges() {
    // On, so the OS "Animation effects" setting (off on CI runners) does not matter.
    let mut settings = Settings::default();
    settings.bar.animations = crate::config::model::Animations::On;
    let (mut logic, window, host, config) = rising_bar(settings);
    // Placed: 28 px below its place, transparent, then rising.
    assert_eq!(window.slides.borrow().first(), Some(&28));
    let mut canvas = RecordingCanvas::new(1.0);
    logic.paint(&mut canvas, SizeF::new(WIDTH, 28.0));
    let dump = canvas.dump();
    assert!(dump.starts_with("opacity 0"), "{dump}");
    let done = Instant::now() + RISE;
    logic.frame(&window, done);
    assert_eq!(window.slides.borrow().last(), Some(&0));
    // Frames that move nothing (e.g. for the hover highlight) leave the window where it is.
    let slides = window.slides.borrow().len();
    logic.frame(&window, done + Duration::from_millis(16));
    assert_eq!(window.slides.borrow().len(), slides);
    let mut canvas = RecordingCanvas::new(1.0);
    logic.paint(&mut canvas, SizeF::new(WIDTH, 28.0));
    assert!(!canvas.dump().starts_with("opacity"));

    // To the top: it slides out downwards first, and only then is redocked.
    let mut top = config.clone();
    top.settings.bar.edge = Edge::Top;
    logic.apply_config(&window, top);
    assert!(!host.calls().contains(&"redock".to_owned()));
    // Still at the bottom: the hairline stays on its top side.
    let mut canvas = RecordingCanvas::new(1.0);
    logic.paint(&mut canvas, SizeF::new(WIDTH, 28.0));
    assert!(
        canvas.dump().contains("fill rect 0,0 1200×1 "),
        "{}",
        canvas.dump()
    );
    logic.frame(&window, Instant::now() + RISE);
    assert_eq!(window.slides.borrow().last(), Some(&28));
    assert!(host.calls().contains(&"redock".to_owned()));
    // Placed at the top: it rises from above.
    window.slides.borrow_mut().clear();
    let mut geometry = logic.geometry.clone().expect("placed");
    geometry.edge = Edge::Top;
    geometry.rect = Rect::new(0, 0, 1200, 28);
    logic.on_event(&window, UiEvent::Placed(geometry));
    assert_eq!(window.slides.borrow().first(), Some(&-28));
}

#[test]
fn an_edge_changed_back_during_the_slide_out_rises_again() {
    let mut settings = Settings::default();
    settings.bar.animations = crate::config::model::Animations::On;
    let (mut logic, window, host, config) = rising_bar(settings);
    logic.frame(&window, Instant::now() + RISE);
    // To the top and back before the slide-out ends: the placement stays at the bottom,
    // so no new one is reported.
    let mut top = config.clone();
    top.settings.bar.edge = Edge::Top;
    logic.apply_config(&window, top);
    logic.apply_config(&window, config);
    logic.frame(&window, Instant::now() + RISE);
    assert!(host.calls().contains(&"redock".to_owned()));
    assert_eq!(logic.shown.target(), 1.0, "it rises");
    logic.frame(&window, Instant::now() + 2 * RISE);
    assert_eq!(window.slides.borrow().last(), Some(&0));
}

#[test]
fn without_animations_the_bar_is_simply_there() {
    let mut settings = Settings::default();
    settings.bar.animations = crate::config::model::Animations::Off;
    let (mut logic, window, host, config) = rising_bar(settings);
    assert_eq!(*window.slides.borrow(), [0]);
    let mut top = config;
    top.settings.bar.edge = Edge::Top;
    logic.apply_config(&window, top);
    assert!(host.calls().contains(&"redock".to_owned()));
}

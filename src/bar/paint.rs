//! Drawing the bar's background (SPEC §5.5, §12): the modern bar's fill and hairline, or the
//! classic bar's skin. The sensors draw themselves on top (`bar/sensors.rs`).

use std::path::Path;

use crate::config::model::{Edge, Settings};
use crate::look::skins;
use crate::look::theme::Theme;
use crate::platform::{Bitmap, Canvas, OsAppearance, PlatformError, RectF, SizeF};

/// The hover highlight multiplies the background opacity by up to this (SPEC §5.5).
const HIGHLIGHT_GAIN: f32 = 1.7;

/// Draws the bar's background into a cleared canvas of `size` logical px: `skin`, the classic
/// theme's rendered skin (`bar/skins`), or else the background colour. `highlight` (0–1) is how
/// far the hover highlight has faded in.
pub(super) fn paint_bar(
    canvas: &mut dyn Canvas,
    size: SizeF,
    theme: &Theme,
    edge: Edge,
    highlight: f32,
    skin: Option<&Bitmap>,
) {
    let background = theme.bar_background;
    let gain = 1.0 + (HIGHLIGHT_GAIN - 1.0) * highlight;
    let opacity = (background.a * gain).min(1.0);
    match skin {
        // The opacity applies to the skin only, not to the sensors (SPEC §12.3).
        Some(skin) => canvas.draw_bitmap(skin, RectF::from_size(size), opacity),
        None => canvas.fill_rect(RectF::from_size(size), background.with_alpha(opacity)),
    }
    if let Some(hairline) = theme.hairline {
        // One px on the edge facing the screen centre (SPEC §12.2), whole physical px thick.
        let thickness = canvas.snap(1.0).max(1.0 / canvas.scale());
        let top = match edge {
            Edge::Bottom => 0.0,
            Edge::Top => size.height - thickness,
        };
        canvas.fill_rect(RectF::new(0.0, top, size.width, top + thickness), hairline);
    }
}

/// A preview of the bar's background as `settings` choose it, e.g. a skin in Settings' gallery
/// (SPEC §12.4): `width` logical px wide and as tall as the bar, on its edge, drawn by the bar's
/// renderer at `scale` with packs from `skins_dir`. Fails where there is no renderer.
pub fn skin_preview(
    settings: &Settings,
    os: OsAppearance,
    skins_dir: &Path,
    width: f32,
    scale: f32,
) -> Result<Bitmap, PlatformError> {
    let theme = Theme::resolve(settings, os);
    let edge = settings.bar.edge;
    let target = skins::Target::of(SizeF::new(width, theme.bar_height as f32), scale, edge);
    // The whole physical px, as the bar window has them.
    let size = SizeF::new(target.width as f32 / scale, target.height as f32 / scale);
    crate::platform::render_offscreen(target.width, target.height, scale, &mut |canvas| {
        let skin = theme
            .skin
            .as_ref()
            .and_then(|spec| skins::render(skins_dir, spec, target));
        paint_bar(
            canvas,
            size,
            &theme,
            edge,
            0.0,
            skin.as_ref().map(|s| &s.bar),
        );
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::{Settings, ThemeId};
    use crate::platform::recording::RecordingCanvas;
    use crate::platform::{Color, OsAppearance};

    #[test]
    fn modern_bar_has_a_hairline_towards_the_screen_centre() {
        let theme = Theme::resolve(&Settings::default(), OsAppearance::default());
        let size = SizeF::new(800.0, 28.0);

        let mut bottom = RecordingCanvas::new(1.0);
        paint_bar(&mut bottom, size, &theme, Edge::Bottom, 0.0, None);
        assert_eq!(
            bottom.dump(),
            "fill rect 0,0 800×28 #1C1C1F/80%\nfill rect 0,0 800×1 #FFFFFF/8%\n"
        );

        // At 150 % the line is two physical px, so it stays crisp.
        let mut top = RecordingCanvas::new(1.5);
        paint_bar(&mut top, size, &theme, Edge::Top, 0.0, None);
        assert_eq!(
            top.dump(),
            "fill rect 0,0 800×28 #1C1C1F/80%\nfill rect 0,26.67 800×1.33 #FFFFFF/8%\n"
        );
    }

    #[test]
    fn hover_highlight_strengthens_the_background() {
        let theme = Theme::resolve(&Settings::default(), OsAppearance::default());
        let mut canvas = RecordingCanvas::new(1.0);
        // Fully in: 80 % × 1.7, capped at 100 %. Halfway at 40 %: 40 % × 1.35 = 54 %.
        paint_bar(
            &mut canvas,
            SizeF::new(100.0, 28.0),
            &theme,
            Edge::Bottom,
            1.0,
            None,
        );
        assert!(canvas.dump().starts_with("fill rect 0,0 100×28 #1C1C1F\n"));
        let s = Settings {
            background: crate::config::model::BackgroundSettings {
                opacity: Some(40),
                ..Default::default()
            },
            ..Settings::default()
        };
        let theme = Theme::resolve(&s, OsAppearance::default());
        let mut canvas = RecordingCanvas::new(1.0);
        paint_bar(
            &mut canvas,
            SizeF::new(100.0, 28.0),
            &theme,
            Edge::Bottom,
            0.5,
            None,
        );
        assert!(
            canvas
                .dump()
                .starts_with("fill rect 0,0 100×28 #1C1C1F/54%\n")
        );
    }

    #[test]
    fn classic_bar_draws_its_skin_with_the_background_opacity() {
        let s = Settings {
            theme: ThemeId::Classic,
            ..Settings::default()
        };
        let theme = Theme::resolve(&s, OsAppearance::default());
        assert_eq!(theme.bar_background, Color::rgba(0x2A, 0x2A, 0x2E, 0.9));
        let size = SizeF::new(100.0, 25.0);
        let spec = theme.skin.as_ref().expect("classic has a skin");
        let target = skins::Target::of(size, 1.5, Edge::Bottom);
        let skin = skins::render(std::path::Path::new("no-skins"), spec, target).map(|s| s.bar);
        let mut canvas = RecordingCanvas::new(1.5);
        paint_bar(&mut canvas, size, &theme, Edge::Bottom, 0.0, skin.as_ref());
        // The skin, and no hairline.
        assert_eq!(canvas.dump(), "bitmap 150×38 into 0,0 100×25 opacity 0.9\n");
        // The hover highlight strengthens the skin as it does the modern background.
        let mut canvas = RecordingCanvas::new(1.5);
        paint_bar(&mut canvas, size, &theme, Edge::Bottom, 1.0, skin.as_ref());
        assert_eq!(canvas.dump(), "bitmap 150×38 into 0,0 100×25 opacity 1\n");
    }

    #[test]
    fn specimen_draws_everywhere() {
        let theme = Theme::resolve(&Settings::default(), OsAppearance::default());
        let engine = crate::platform::recording::FixedMetrics;
        let mut canvas = RecordingCanvas::new(1.0);
        let images = specimen::images(28, 1.0, theme.accent);
        specimen::paint(&mut canvas, &engine, &theme, &images);
        let dump = canvas.dump();
        assert!(dump.contains("text \"CPU\" Segoe UI Variable Text 600 10px"));
        assert!(dump.contains("bitmap "), "{dump}");
        let pushes = dump
            .lines()
            .filter(|l| l.starts_with("clip") || l.starts_with("opacity"));
        assert_eq!(pushes.count(), dump.lines().filter(|l| *l == "pop").count());
    }

    /// A sample of every canvas operation in bar-sized pieces, for reviewing the renderer in
    /// CI's screenshots beside the bar the sensors draw.
    pub(super) mod specimen {
        use std::f32::consts::TAU;
        use std::path::Path;

        use crate::look::theme::{Inactive, Theme};
        use crate::platform::{
            Bitmap, Brush, Canvas, Color, PointF, RectF, Shape, SizeF, TextEngine, TextStyle,
            load_image,
        };

        pub const WIDTH: f32 = 900.0;

        /// The SVG app icon, also tinted with `tint` (SPEC §12.3), and the PNG icon where raster
        /// decoding exists; two px less than `bar_height` logical px tall.
        pub fn images(bar_height: u32, scale: f32, tint: Color) -> Vec<Bitmap> {
            let icons = Path::new(env!("CARGO_MANIFEST_DIR")).join("icons");
            let px = ((bar_height - 2) as f32 * scale).round() as u32;
            let mut images = Vec::new();
            if let Ok(svg) = load_image(&icons.join("app-icon.svg"), Some(px)) {
                images.push(svg.tinted(tint));
                images.push(svg);
            }
            if let Ok(png) = load_image(&icons.join("128x128.png"), Some(px)) {
                images.push(png);
            }
            images
        }

        /// Draws the specimen on a desktop-like backdrop, `WIDTH` × bar height.
        pub fn paint(
            canvas: &mut dyn Canvas,
            text: &dyn TextEngine,
            theme: &Theme,
            images: &[Bitmap],
        ) {
            let h = theme.bar_height as f32;
            let stop = |offset, color| crate::platform::canvas::GradientStop { offset, color };
            // The desktop behind the translucent bar.
            canvas.fill(
                &Shape::Rect(RectF::new(0.0, 0.0, WIDTH, h)),
                &Brush::Linear {
                    start: PointF::new(0.0, 0.0),
                    end: PointF::new(WIDTH, 0.0),
                    stops: vec![
                        stop(0.0, Color::rgba(0x1F, 0x4E, 0x8C, 1.0)),
                        stop(0.5, Color::rgba(0xD0, 0x7A, 0x3A, 1.0)),
                        stop(1.0, Color::rgba(0x2E, 0x7D, 0x32, 1.0)),
                    ],
                },
            );
            let size = SizeF::new(WIDTH, h);
            let edge = crate::config::model::Edge::Bottom;
            let skin = theme.skin.as_ref().and_then(|spec| {
                crate::look::skins::render(
                    std::path::Path::new("no-skins"),
                    spec,
                    crate::look::skins::Target::of(size, canvas.scale(), edge),
                )
                .map(|s| s.bar)
            });
            super::super::paint_bar(canvas, size, theme, edge, 0.0, skin.as_ref());
            let fg = theme.sensor_text;
            let small = TextStyle::new(&theme.sensor_font, theme.small_size).tabular();
            let mut x = theme.pad;

            // Two-line block: label above value, centred as a block (SPEC §8.3).
            let line = theme.small_size + 1.0;
            let top = canvas.snap((h - 2.0 * line) / 2.0);
            let label = text.layout("CPU", &small);
            let value = text.layout("37%", &small);
            let baseline = |canvas: &dyn Canvas, row: f32, layout: &crate::platform::TextLayout| {
                canvas.snap(top + row * line + theme.small_size) - layout.baseline()
            };
            // Meter (SPEC §8.3): 4 px bar, background 31 %, fill 59 %.
            let meter = RectF::new(x, 3.0, x + 4.0, h - 3.0);
            canvas.fill_rect(meter, fg.faded(0.31));
            let fill_top = meter.bottom - 0.37 * meter.height();
            canvas.fill_rect(
                RectF::new(meter.left, fill_top, meter.right, meter.bottom),
                fg.faded(0.59),
            );
            x += 8.0;
            canvas.draw_text(&label, PointF::new(x, baseline(canvas, 0.0, &label)), fg);
            let column = x + 24.0;
            canvas.draw_text(
                &value,
                PointF::new(column - value.width(), baseline(canvas, 1.0, &value)),
                fg,
            );
            x = column + theme.gap;

            // Graph (SPEC §8.3): one column per px, dynamic colours, grid and background.
            let plot = RectF::new(x, 2.0, x + 60.0, h - 2.0);
            canvas.fill_rect(plot, Color::BLACK.faded(0.3));
            let mut gx = plot.left;
            while gx < plot.right {
                canvas.draw_line(
                    PointF::new(gx + 0.5, plot.top),
                    PointF::new(gx + 0.5, plot.bottom),
                    &Color::rgba(0xA0, 0xFF, 0x9B, 0.15).into(),
                    1.0,
                );
                gx += 5.0;
            }
            for i in 0..60 {
                let v = 0.5 + 0.45 * (i as f32 / 60.0 * TAU * 1.5).sin();
                let color = if v < 0.5 {
                    Color::rgba((v * 510.0).round() as u8, 255, 0, 0.55)
                } else {
                    Color::rgba(255, ((1.0 - v) * 510.0).round() as u8, 0, 0.55)
                };
                let left = plot.left + i as f32;
                canvas.fill_rect(
                    RectF::new(
                        left,
                        plot.bottom - v * plot.height(),
                        left + 1.0,
                        plot.bottom,
                    ),
                    color,
                );
            }
            x = plot.right + theme.gap;

            // Clock in the large size with tabular numerals, then medium.
            let large = TextStyle::new(&theme.sensor_font, theme.large_size).tabular();
            let clock = text.layout("12:45", &large);
            let clock_y = canvas.snap((h - clock.height()) / 2.0);
            canvas.draw_text(&clock, PointF::new(x, clock_y), fg);
            x += clock.width() + 4.0;
            let medium = TextStyle::new(&theme.sensor_font, theme.medium_size);
            let date = text.layout("Mon 12 Oct", &medium);
            canvas.draw_text(
                &date,
                PointF::new(x, canvas.snap((h - date.height()) / 2.0)),
                fg,
            );
            x += date.width() + theme.gap;

            // Ellipsis at a maximum width.
            let cut = text.layout(
                "A text sensor that is far too long",
                &TextStyle::new(&theme.sensor_font, 12.0).max_width(90.0),
            );
            canvas.draw_text(
                &cut,
                PointF::new(x, canvas.snap((h - cut.height()) / 2.0)),
                fg,
            );
            x += 90.0 + theme.gap;

            // Images; the second at the inactive opacity (SPEC §8.5).
            let inactive = match theme.inactive {
                Inactive::Opacity(opacity) => opacity,
                Inactive::Colorize(_) => 1.0,
            };
            for (i, image) in images.iter().enumerate() {
                let size = (h - 2.0) * image.width() as f32 / image.height() as f32;
                let dest = RectF::new(x, 1.0, x + size, h - 1.0);
                canvas.draw_bitmap(image, dest, if i == 1 { inactive } else { 1.0 });
                x += size + 4.0;
            }
            x += theme.gap - 4.0;

            // Popup shapes: rounded rectangle with a pointer triangle, outlined.
            let bubble = RectF::new(x, 2.0, x + 70.0, h - 9.0);
            canvas.fill(
                &Shape::RoundedRect {
                    rect: bubble,
                    radius: 6.0,
                },
                &theme.popup_background.into(),
            );
            let mid = bubble.left + 35.0;
            canvas.fill(
                &Shape::Polygon(vec![
                    PointF::new(mid - 6.5, bubble.bottom),
                    PointF::new(mid + 6.5, bubble.bottom),
                    PointF::new(mid, bubble.bottom + 7.0),
                ]),
                &theme.popup_background.into(),
            );
            canvas.stroke(
                &Shape::RoundedRect {
                    rect: bubble.inset(0.5),
                    radius: 6.0,
                },
                &theme.accent.into(),
                1.0,
            );
            let popup = TextStyle::new(&theme.popup_font, theme.popup_font_size);
            let hint = text.layout("Hint", &popup);
            canvas.draw_text(
                &hint,
                PointF::new(
                    bubble.left + 6.0,
                    canvas.snap(bubble.top + (bubble.height() - hint.height()) / 2.0),
                ),
                theme.popup_strong_text,
            );
            x = bubble.right + theme.gap;

            // Gloss skin piece: base colour, white gradient over the upper half (SPEC §12.4).
            let gloss = RectF::new(x, 0.0, x + 60.0, h);
            canvas.fill_rect(gloss, Color::rgba(0x2A, 0x2A, 0x2E, 1.0));
            canvas.fill(
                &Shape::Rect(RectF::new(gloss.left, 0.0, gloss.right, h / 2.0)),
                &Brush::Linear {
                    start: PointF::new(0.0, 0.0),
                    end: PointF::new(0.0, h / 2.0),
                    stops: vec![
                        stop(0.0, Color::WHITE.with_alpha(0.30)),
                        stop(1.0, Color::WHITE.with_alpha(0.05)),
                    ],
                },
            );
            x = gloss.right + theme.gap;

            // Clipping: the text is cut at the clip's right edge.
            let clipped = text.layout("Clipped", &small);
            canvas.push_clip(RectF::new(x, 0.0, x + clipped.width() / 2.0, h));
            canvas.draw_text(
                &clipped,
                PointF::new(x, canvas.snap((h - clipped.height()) / 2.0)),
                fg,
            );
            canvas.pop();
            x += clipped.width() + theme.gap;

            // A layer at 40 %: overlapping shapes fade as one.
            canvas.push_opacity(0.4);
            canvas.fill_rect(RectF::new(x, 4.0, x + 16.0, h - 4.0), Color::WHITE);
            canvas.fill(
                &Shape::Polygon(vec![
                    PointF::new(x + 8.0, 2.0),
                    PointF::new(x + 24.0, h / 2.0),
                    PointF::new(x + 8.0, h - 2.0),
                ]),
                &Color::rgba(0xFF, 0x40, 0x40, 1.0).into(),
            );
            canvas.pop();
            x += 28.0;

            // Accent line, like the drop placer (SPEC §12.2).
            canvas.fill_rect(RectF::new(x, 2.0, x + 2.0, h - 2.0), theme.accent);
        }
    }

    /// Made-up samples for the screenshots of the system sensors: a plot's worth of each
    /// topic the sensors subscribe to, varying over time.
    #[cfg(windows)]
    mod samples {
        use std::sync::Arc;

        use crate::bar::sensors::{BarSensors, Env};
        use crate::sampler::{
            CpuSample, Delivery, DiskSample, MemorySample, NetSample, Sample, SwapSample, Topic,
            UptimeSample,
        };

        const GB: u64 = 1 << 30;

        fn sample(topic: &Topic, i: usize) -> Sample {
            let wave = |period: f32, phase: f32| (i as f32 / period + phase).sin();
            match topic {
                Topic::Cpu => {
                    let total = 35.0 + 30.0 * wave(7.0, 0.0);
                    Sample::Cpu(CpuSample {
                        total,
                        user: total * 0.6,
                        kernel: total * 0.3,
                        idle: 100.0 - total,
                        cores: (0..8).map(|c| 50.0 + 45.0 * wave(5.0, c as f32)).collect(),
                        logical: 8,
                    })
                }
                Topic::Memory => {
                    let total = 32 * GB;
                    let load = 45.0 + 8.0 * wave(20.0, 0.0);
                    let used = (total as f64 * f64::from(load) / 100.0) as u64;
                    Sample::Memory(MemorySample {
                        total,
                        available: total - used,
                        used,
                        load,
                    })
                }
                Topic::Swap => {
                    let total = 8 * GB;
                    let load = 12.0 + 6.0 * wave(15.0, 1.0);
                    Sample::Swap(SwapSample {
                        total,
                        used: (total as f64 * f64::from(load) / 100.0) as u64,
                        load,
                    })
                }
                Topic::Disk(root) => Sample::Disk(if root.starts_with('Q') {
                    DiskSample::default()
                } else {
                    DiskSample {
                        present: true,
                        total: 953 * GB,
                        free: 241 * GB,
                    }
                }),
                Topic::Volumes => Sample::Volumes(crate::platform::current().volumes()),
                // What the path is on this machine (the launchers' targets).
                Topic::Path(path) => Sample::Path(crate::sampler::PathSample {
                    kind: crate::platform::current().path_kind(path),
                }),
                Topic::Net(_) => Sample::Net(NetSample {
                    present: true,
                    id: "WIFI".into(),
                    alias: "Wi-Fi".into(),
                    up: true,
                    rx_rate: f64::from(1.3e6 * (1.0 + wave(6.0, 0.0)).powi(2)),
                    tx_rate: f64::from(9e4 * (1.0 + wave(4.0, 2.0))),
                    ..NetSample::default()
                }),
                // Battery 1 is on battery power with 2:41 left; battery 2 is not there.
                Topic::Battery(index) => Sample::Battery(crate::sampler::BatterySample {
                    present: *index == 0,
                    name: "Battery".into(),
                    percent: 73.0,
                    ac_online: false,
                    state: crate::platform::BatteryState::Discharging,
                    time_to_empty: Some(std::time::Duration::from_secs(161 * 60)),
                    time_to_full: None,
                }),
                Topic::Batteries => Sample::Batteries(vec!["Battery".into()]),
                Topic::Wifi => Sample::Wifi(crate::platform::WifiStatus {
                    available: true,
                    radio_on: true,
                    connected: true,
                    ssid: Some("HomeNet".into()),
                    quality: 78,
                    rssi_dbm: Some(-58),
                    frequency_mhz: Some(5180),
                    link_mbps: Some(1201),
                    phy: Some("802.11ax"),
                    location_denied: false,
                }),
                Topic::RecycleBin => Sample::RecycleBin(crate::platform::RecycleBin {
                    items: 12,
                    bytes: 431 << 20,
                }),
                Topic::Uptime => Sample::Uptime(UptimeSample {
                    boot_unix_ms: 0,
                    uptime_secs: 3 * 86_400 + 4 * 3600 + 12 * 60,
                    read_at: std::time::Instant::now(),
                }),
            }
        }

        pub fn feed(sensors: &mut BarSensors, env: &Env) {
            feed_seconds(sensors, env, 0..150, std::time::Instant::now());
        }

        /// Delivers the samples of `seconds`, one a second from `start`, as the sampler would
        /// read them.
        pub fn feed_seconds(
            sensors: &mut BarSensors,
            env: &Env,
            seconds: std::ops::Range<usize>,
            start: std::time::Instant,
        ) {
            let mut topics: Vec<Topic> = Vec::new();
            for request in sensors.subscriptions().into_values().flatten() {
                if !topics.contains(&request.topic) {
                    topics.push(request.topic);
                }
            }
            for i in seconds {
                let now = start + std::time::Duration::from_secs(i as u64);
                for topic in &topics {
                    let delivery = Delivery {
                        topic: topic.clone(),
                        sample: Arc::new(sample(topic, i)),
                        to: None,
                        pushed: false,
                    };
                    sensors.deliver(&delivery, env, now);
                }
            }
        }
    }

    /// The full bar redraw (SPEC §14.1, ≤ 2 ms): the first-run layout on a bar 2560 px wide at
    /// 100 %, each frame after a second's samples arrived, drawn as the bar draws one (the
    /// sensors that changed measure again, then the background and every sensor) by Direct2D
    /// with a renderer kept across frames. CI's budget check reads the median.
    #[cfg(windows)]
    #[test]
    #[ignore = "measures a budget; run with --ignored"]
    fn budget_bar_redraw() {
        use std::time::{Duration, Instant};

        use crate::bar::sensors::{BarSensors, Env};
        use crate::platform::{FrameRenderer, text_engine};
        use crate::sensors::LayoutCtx;
        use crate::test_util::{median, record_budget};

        const FRAMES: usize = 200;
        const WIDTH: u32 = 2560;
        let settings = Settings::default();
        let theme = Theme::resolve(&settings, OsAppearance::default());
        let platform = crate::platform::current();
        let text = text_engine();
        let mut sensors = BarSensors::default();
        sensors.sync(
            &crate::config::Config::first_run("1.0.0").layout,
            &Default::default(),
            &platform.capabilities(),
            crate::sensors::registry::registry(),
        );
        let locale = platform.locale(settings.date_locale);
        let numbers = crate::sensors::format::Numbers::from(&locale);
        let env = Env {
            settings: &settings,
            platform: platform.as_ref(),
            alt: false,
            animate: false,
            locale: &locale,
            numbers,
            changes: &crate::sampler::ChangeSink::NONE,
        };
        let start = Instant::now();
        sensors.wake(start, &env);
        // Graphs with a full plot.
        samples::feed_seconds(&mut sensors, &env, 0..150, start);
        let cx = LayoutCtx {
            theme: &theme,
            text: text.as_ref(),
            scale: 1.0,
            locale: &locale,
            numbers,
            icons: &crate::sensors::DirectIcons(platform.as_ref()),
        };
        let size = SizeF::new(WIDTH as f32, theme.bar_height as f32);
        let mut renderer = FrameRenderer::new().expect("a renderer");
        let mut times = Vec::with_capacity(FRAMES);
        for i in 150..150 + FRAMES {
            samples::feed_seconds(&mut sensors, &env, i..i + 1, start);
            let began = Instant::now();
            renderer
                .draw(WIDTH, theme.bar_height, 1.0, &mut |canvas| {
                    sensors.update_layout(&cx, size.width);
                    paint_bar(canvas, size, &theme, Edge::Bottom, 0.0, None);
                    sensors.draw(canvas, &theme, size.height);
                })
                .expect("drawn");
            times.push(began.elapsed());
        }
        record_budget(
            "redraw",
            "Full bar redraw (2560 px)",
            median(&mut times),
            Duration::from_millis(2),
            FRAMES,
        );
    }

    /// The hint shown after its delay (SPEC §14.1, ≤ 30 ms): as the bar shows one when its
    /// delay is over, the first-run Date sensor's calendar is asked for, laid out, framed above
    /// the sensor and drawn by Direct2D with a renderer kept across hints, as the hint window
    /// keeps its own. CI's budget check reads the median.
    #[cfg(windows)]
    #[test]
    #[ignore = "measures a budget; run with --ignored"]
    fn budget_hint() {
        use std::time::{Duration, Instant};

        use crate::bar::hint::Prepared;
        use crate::bar::placement::Anchor;
        use crate::bar::sensors::{BarSensors, Env};
        use crate::platform::{FrameRenderer, RectF, text_engine};
        use crate::sensors::{HintContent, LayoutCtx};
        use crate::test_util::{median, record_budget};

        const RUNS: usize = 100;
        let settings = Settings::default();
        let theme = Theme::resolve(&settings, OsAppearance::default());
        let platform = crate::platform::current();
        let text = text_engine();
        let layout = crate::config::Config::first_run("1.0.0").layout;
        let date = layout
            .all()
            .find(|s| s.kind == "date")
            .expect("a Date sensor")
            .id
            .clone();
        let mut sensors = BarSensors::default();
        sensors.sync(
            &layout,
            &Default::default(),
            &platform.capabilities(),
            crate::sensors::registry::registry(),
        );
        let locale = platform.locale(settings.date_locale);
        let numbers = crate::sensors::format::Numbers::from(&locale);
        let env = Env {
            settings: &settings,
            platform: platform.as_ref(),
            alt: false,
            animate: false,
            locale: &locale,
            numbers,
            changes: &crate::sampler::ChangeSink::NONE,
        };
        sensors.wake(Instant::now(), &env);
        let cx = LayoutCtx {
            theme: &theme,
            text: text.as_ref(),
            scale: 1.0,
            locale: &locale,
            numbers,
            icons: &crate::sensors::DirectIcons(platform.as_ref()),
        };
        sensors.update_layout(&cx, 1920.0);
        let (x, width) = sensors.place_of(&date).expect("placed");
        let bar = RectF::new(0.0, 1052.0, 1920.0, 1080.0);
        let anchor = Anchor {
            rect: RectF::new(x, bar.top, x + width, bar.bottom),
            bar,
            monitor: RectF::new(0.0, 0.0, 1920.0, 1080.0),
            edge: Edge::Bottom,
        };
        let mut renderer = FrameRenderer::new().expect("a renderer");
        let mut times = Vec::with_capacity(RUNS);
        for _ in 0..RUNS {
            let began = Instant::now();
            let content = sensors.hint(&date, &env).expect("a hint");
            assert!(matches!(content, HintContent::Rich(_)), "a calendar");
            let hint = Prepared::new(content, text.as_ref(), &anchor, &theme).expect("not empty");
            let window = hint.frame.window;
            let (w, h) = (
                window.width().round() as u32,
                window.height().round() as u32,
            );
            renderer
                .draw(w, h, 1.0, &mut |canvas| hint.paint(canvas, &theme))
                .expect("drawn");
            times.push(began.elapsed());
        }
        record_budget(
            "hint",
            "Hint after its delay (calendar)",
            median(&mut times),
            Duration::from_millis(30),
            RUNS,
        );
    }

    /// The Folder Menu's first paint for a folder of 500 items (SPEC §14.1, ≤ 200 ms): from the
    /// click's popup to its first frame showing the items, as the bar opens one: the folder is
    /// listed and sorted on a worker while the first layout waits for it, then the popup is laid
    /// out, placed above its sensor and drawn by Direct2D, its shell icons still loading. Each
    /// run lists the folder anew. CI's budget check reads the median.
    #[cfg(windows)]
    #[test]
    #[ignore = "measures a budget; run with --ignored"]
    fn budget_folder_menu() {
        use std::time::{Duration, Instant};

        use crate::bar::placement::Anchor;
        use crate::bar::popup::{self, Open};
        use crate::platform::{FrameRenderer, RectF, UiWaker, text_engine};
        use crate::sensors::common::folder_browser::{FolderBrowser, Options};
        use crate::sensors::popup::{Popup, PopupCtx, PopupReply};
        use crate::sensors::{Fetched, IconSource};
        use crate::test_util::{median, record_budget};

        /// The bar's icon loader before it has any of them.
        struct Loading;
        impl IconSource for Loading {
            fn shell_icon(&self, _path: &str, _px: u32) -> Fetched {
                Fetched::Loading
            }
            fn forget(&self, _path: &str) {}
        }

        const RUNS: usize = 20;
        // 50 folders and 450 files with numbered names, which sort naturally.
        let dir = tempfile::tempdir().expect("temp dir");
        for i in 0..50 {
            std::fs::create_dir(dir.path().join(format!("Project {i}"))).expect("folder");
        }
        for i in 0..450 {
            let name = match i % 3 {
                0 => format!("Report {i}.docx"),
                1 => format!("photo_{i:04}.jpg"),
                _ => format!("Notes {i}.txt"),
            };
            std::fs::write(dir.path().join(name), b"x").expect("file");
        }
        let folder = dir.path().to_string_lossy().into_owned();

        let settings = Settings::default();
        let theme = Theme::resolve(&settings, OsAppearance::default());
        let platform = crate::platform::current();
        let text = text_engine();
        let waker = UiWaker::new(|| {});
        let bar = RectF::new(0.0, 1052.0, 1920.0, 1080.0);
        let anchor = Anchor {
            rect: RectF::new(300.0, bar.top, 321.0, bar.bottom),
            bar,
            monitor: RectF::new(0.0, 0.0, 1920.0, 1080.0),
            edge: Edge::Bottom,
        };
        let room = popup::room(&anchor);
        let mut renderer = FrameRenderer::new().expect("a renderer");
        let mut times = Vec::with_capacity(RUNS);
        for _ in 0..RUNS {
            let cx = PopupCtx {
                theme: &theme,
                text: text.as_ref(),
                scale: 1.0,
                platform: &platform,
                icons: &Loading,
                waker: &waker,
                room,
                monitor_height: anchor.monitor.height(),
                edge: Edge::Bottom,
                free_left: false,
                animate: false,
                numbers: Default::default(),
                now: Instant::now(),
            };
            let began = Instant::now();
            let mut browser = FolderBrowser::new(Options {
                folders: vec![folder.clone()],
                title: "Budget".into(),
                show_hidden: false,
                width: 240.0,
                height: 320.0,
            });
            let mut size = browser.layout(&cx);
            // A listing slower than the first layout's wait arrives later.
            while !browser.listed() {
                std::thread::sleep(Duration::from_millis(1));
                if browser.poll(&cx) == PopupReply::Relayout {
                    size = browser.layout(&cx);
                }
            }
            let mut open = Open {
                owner: "f".into(),
                popup: Box::new(browser),
                frame: None,
                room,
                monitor_height: anchor.monitor.height(),
                free_left: false,
                captured: false,
            };
            open.place(size, &anchor, &theme);
            let frame = open.frame.expect("placed");
            let (w, h) = (
                frame.window.width().round() as u32,
                frame.window.height().round() as u32,
            );
            renderer
                .draw(w, h, 1.0, &mut |canvas| popup::paint(canvas, &open, &theme))
                .expect("drawn");
            times.push(began.elapsed());
        }
        record_budget(
            "folder-menu",
            "Folder Menu, 500 items, to first paint",
            median(&mut times),
            Duration::from_millis(200),
            RUNS,
        );
    }

    /// Renders the bar with the real renderer to `target/screenshots/bar-*.png` (CI uploads
    /// them): plain bars per theme and every built-in skin, the canvas specimen, the first-run
    /// layout, the General sensors, every system sensor with made-up samples and the launchers,
    /// at 100 % and 150 %.
    #[cfg(windows)]
    #[test]
    #[ignore = "writes PNGs to target/screenshots; run with --ignored"]
    fn render_bar() {
        use crate::bar::sensors::BarSensors;
        use crate::config::model::{Layout, SensorInstance};
        use crate::platform::{render_offscreen, text_engine};
        use crate::sensors::LayoutCtx;
        use crate::test_util::{screenshot_path, write_png};

        // A bar window `width` × `height` logical px at `scale`: its logical size from the whole
        // physical px it has (25 px at 150 % is 38), and those.
        let window = |width: f32, height: u32, scale: f32| {
            let (w, h) = (
                (width * scale).round() as u32,
                (height as f32 * scale).round() as u32,
            );
            (SizeF::new(w as f32 / scale, h as f32 / scale), w, h)
        };
        let save = |name: &str, bitmap: &crate::platform::Bitmap| {
            write_png(
                &screenshot_path(name),
                bitmap.width(),
                bitmap.height(),
                &bitmap.to_rgba(),
            );
        };
        let dark = OsAppearance::default();
        let light = OsAppearance {
            dark: false,
            ..dark
        };
        let classic = Settings {
            theme: ThemeId::Classic,
            ..Settings::default()
        };
        // A skin pack, made here (SPEC §12.4): a blue tile with a 9 px shadow band, drawn for a
        // 25 px bar, with a groove at its right edge so the tiling shows.
        let skins_dir = tempfile::tempdir().expect("temp dir");
        let pack_dir = skins_dir.path().join("blue-steel");
        std::fs::create_dir_all(&pack_dir).expect("pack folder");
        std::fs::write(
            pack_dir.join("skin.json"),
            r#"{ "name": "Blue Steel", "image": "tile.png", "shadowBand": 9 }"#,
        )
        .expect("skin.json");
        let tile: Vec<u8> = (0..34u32)
            .flat_map(|y| (0..24u32).map(move |x| (x, y)))
            .flat_map(|(x, y)| match y {
                // The band: black, darkest next to the bar.
                0..9 => [0, 0, 0, (y * y * 2) as u8],
                9 => [0xA8, 0xC8, 0xF0, 255],
                _ if x == 23 => [0x10, 0x20, 0x38, 255],
                _ => {
                    let t = (y - 10) as f32 / 23.0;
                    let mix = |a: f32, b: f32| (a + (b - a) * t) as u8;
                    [mix(90.0, 31.0), mix(143.0, 63.0), mix(208.0, 111.0), 255]
                }
            })
            .collect();
        write_png(&pack_dir.join("tile.png"), 24, 34, &tile);
        let skin_of = |theme: &Theme, size: SizeF, scale: f32, edge: Edge| {
            theme.skin.as_ref().and_then(|spec| {
                skins::render(skins_dir.path(), spec, skins::Target::of(size, scale, edge))
            })
        };
        let mut bars = vec![
            (
                "bar-modern-dark".to_owned(),
                Settings::default(),
                dark,
                Edge::Bottom,
                1.0,
            ),
            (
                "bar-modern-light-top".to_owned(),
                Settings::default(),
                light,
                Edge::Top,
                1.0,
            ),
            (
                "bar-classic".to_owned(),
                classic.clone(),
                dark,
                Edge::Bottom,
                1.0,
            ),
        ];
        // Every built-in skin (SPEC §12.4), also at 150 %, where its lines and textures follow
        // the scale; Glass on a top bar, where its highlight is at the bottom; Gloss tinted.
        let skinned = |id: &str, color: Option<&str>| {
            let mut s = classic.clone();
            s.classic.skin = id.into();
            s.background.color = color.map(Into::into);
            s
        };
        for skin in skins::BuiltIn::ALL {
            let settings = skinned(skin.id(), None);
            let name = format!("bar-classic-{}", skin.id());
            bars.push((name.clone(), settings.clone(), dark, Edge::Bottom, 1.0));
            bars.push((format!("{name}@1.5x"), settings, dark, Edge::Bottom, 1.5));
        }
        bars.push((
            "bar-classic-glass-top".to_owned(),
            skinned("glass", None),
            dark,
            Edge::Top,
            1.0,
        ));
        bars.push((
            "bar-classic-gloss-blue".to_owned(),
            skinned("gloss", Some("#3A6EA5")),
            dark,
            Edge::Bottom,
            1.0,
        ));
        // The pack on both edges, at 150 %, on a 32 px bar (its bar part scaled to fit), and
        // tinted; a missing pack (Gloss).
        let pack = skinned("blue-steel", None);
        let mut tall = pack.clone();
        tall.bar.height = Some(32);
        for (name, settings, edge, scale) in [
            ("bar-classic-pack", pack.clone(), Edge::Bottom, 1.0),
            ("bar-classic-pack-top", pack.clone(), Edge::Top, 1.0),
            ("bar-classic-pack@1.5x", pack.clone(), Edge::Bottom, 1.5),
            ("bar-classic-pack-32", tall, Edge::Bottom, 1.0),
            (
                "bar-classic-pack-tinted",
                skinned("blue-steel", Some("#A5703A")),
                Edge::Bottom,
                1.0,
            ),
            (
                "bar-classic-missing",
                skinned("gone", None),
                Edge::Bottom,
                1.0,
            ),
        ] {
            bars.push((name.to_owned(), settings, dark, edge, scale));
        }
        for (name, settings, os, edge, scale) in bars {
            let theme = Theme::resolve(&settings, os);
            let (size, width, height) = window(400.0, theme.bar_height, scale);
            let skin = skin_of(&theme, size, scale, edge);
            let bitmap = render_offscreen(width, height, scale, &mut |canvas| {
                paint_bar(
                    canvas,
                    size,
                    &theme,
                    edge,
                    0.0,
                    skin.as_ref().map(|s| &s.bar),
                )
            })
            .expect("rendered");
            save(&name, &bitmap);
        }
        // The pack's shadow band beside the bar, on the side facing the screen centre.
        for (name, edge, scale) in [
            ("bar-classic-pack-band", Edge::Bottom, 1.0),
            ("bar-classic-pack-band-top", Edge::Top, 1.0),
            ("bar-classic-pack-band@1.5x", Edge::Bottom, 1.5),
        ] {
            let theme = Theme::resolve(&pack, dark);
            let size = SizeF::new(400.0, theme.bar_height as f32);
            let skin = skin_of(&theme, size, scale, edge).expect("drawn");
            let band = skin.band.clone().expect("a band");
            let (width, bar, shadow) = (skin.bar.width(), skin.bar.height(), band.height());
            let (band_top, bar_top) = match edge {
                Edge::Bottom => (0, shadow),
                Edge::Top => (bar, 0),
            };
            let rect = |top: u32, height: u32| {
                crate::platform::RectF::new(0.0, top as f32, width as f32, (top + height) as f32)
            };
            let bitmap = render_offscreen(width, bar + shadow, 1.0, &mut |canvas| {
                canvas.draw_bitmap(&band, rect(band_top, shadow), 1.0);
                canvas.draw_bitmap(&skin.bar, rect(bar_top, bar), theme.bar_background.a);
            })
            .expect("rendered");
            save(name, &bitmap);
        }
        // The bar's own shadow (SPEC §5.5) over a light desktop, where it shows best: modern on
        // both edges and classic at 150 %.
        for (name, settings, os, edge, scale) in [
            ("bar-shadow", Settings::default(), dark, Edge::Bottom, 1.0),
            ("bar-shadow-top", Settings::default(), light, Edge::Top, 1.0),
            (
                "bar-shadow-classic@1.5x",
                skinned("gloss", None),
                dark,
                Edge::Bottom,
                1.5,
            ),
        ] {
            let theme = Theme::resolve(&settings, os);
            let (size, width, bar) = window(400.0, theme.bar_height, scale);
            let skin = skin_of(&theme, size, scale, edge);
            let drawn = render_offscreen(width, bar, scale, &mut |canvas| {
                paint_bar(
                    canvas,
                    size,
                    &theme,
                    edge,
                    0.0,
                    skin.as_ref().map(|s| &s.bar),
                )
            })
            .expect("rendered");
            let band =
                crate::bar::shadow::band(width, scale, edge == Edge::Bottom).expect("a shadow");
            let depth = band.height();
            let (band_top, bar_top) = match edge {
                Edge::Bottom => (0, depth),
                Edge::Top => (bar, 0),
            };
            let rect = |top: u32, height: u32| {
                crate::platform::RectF::new(0.0, top as f32, width as f32, (top + height) as f32)
            };
            let bitmap = render_offscreen(width, bar + depth, 1.0, &mut |canvas| {
                canvas.fill_rect(
                    rect(0, bar + depth),
                    crate::platform::Color::rgba(0xD8, 0xDC, 0xE0, 1.0),
                );
                canvas.draw_bitmap(&band, rect(band_top, depth), 1.0);
                canvas.draw_bitmap(&drawn, rect(bar_top, bar), 1.0);
            })
            .expect("rendered");
            save(name, &bitmap);
        }

        let text = text_engine();
        for (name, settings, os) in [
            ("bar-specimen", Settings::default(), dark),
            ("bar-specimen-light", Settings::default(), light),
            ("bar-specimen-classic", classic.clone(), dark),
        ] {
            let theme = Theme::resolve(&settings, os);
            for (suffix, scale) in [("", 1.0), ("@1.5x", 1.5)] {
                let images = specimen::images(theme.bar_height, scale, theme.accent);
                assert_eq!(images.len(), 3, "SVG and PNG decoded");
                let width = (specimen::WIDTH * scale).round() as u32;
                let height = (theme.bar_height as f32 * scale).round() as u32;
                let bitmap = render_offscreen(width, height, scale, &mut |canvas| {
                    specimen::paint(canvas, text.as_ref(), &theme, &images)
                })
                .expect("rendered");
                save(&format!("{name}{suffix}"), &bitmap);
            }
        }

        // Sensors (SPEC §16): the first-run layout and the General sensors.
        let mut general = Layout::default();
        let entry = |id: &str, kind: &str, props: serde_json::Value| SensorInstance {
            id: id.into(),
            kind: kind.into(),
            props: props.as_object().cloned().unwrap_or_default(),
        };
        general.left = vec![
            entry("a", "text", serde_json::json!({})),
            entry("b", "spacer", serde_json::json!({ "showDivider": true })),
            entry(
                "c",
                "text",
                serde_json::json!({ "text": "Shadow", "shadow": true }),
            ),
            entry(
                "d",
                "spacer",
                serde_json::json!({ "showDivider": true, "width": 21 }),
            ),
            entry(
                "e",
                "text",
                serde_json::json!({ "text": "Bold italic 18", "fontStyle": "boldItalic", "fontSize": 18 }),
            ),
        ];
        general.center = vec![entry("f", "image", serde_json::json!({}))];
        general.right = vec![
            entry(
                "g",
                "image",
                serde_json::json!({ "color": { "mode": "custom", "color": "#20B7FF" } }),
            ),
            entry("h", "image", serde_json::json!({ "opacity": 40 })),
        ];
        // Every system sensor, with made-up samples: CPU, memory and dual graph on the left,
        // network, uptime, batteries (the last one absent) and Wi-Fi in the centre, swap, disks
        // (Q:\ is absent) and the Recycle Bin on the right. The CI runner has neither a battery
        // nor Wi-Fi, so those capabilities are assumed.
        let mut system = Layout::default();
        let dynamic = serde_json::json!({ "mode": "dynamic", "color": "#FFFFFF" });
        system.left = vec![
            entry("s1", "cpu-meter", serde_json::json!({ "color": dynamic })),
            entry(
                "s2",
                "cpu-graph",
                serde_json::json!({ "bkOpacity": 40, "gridOpacity": 25 }),
            ),
            entry("s3", "cpu-graph-dual", serde_json::json!({})),
            entry("s4", "memory-meter", serde_json::json!({})),
            entry(
                "s5",
                "memory-graph",
                serde_json::json!({ "graphColor": dynamic }),
            ),
        ];
        system.center = vec![
            entry("s6", "network-graph", serde_json::json!({ "width": 80 })),
            entry("s7", "uptime", serde_json::json!({})),
            entry("s13", "battery", serde_json::json!({})),
            entry(
                "s14",
                "battery",
                serde_json::json!({ "onBatteryShow": "time" }),
            ),
            entry("s15", "battery", serde_json::json!({ "battery": 1 })),
            entry("s16", "wifi", serde_json::json!({})),
            entry("s17", "wifi", serde_json::json!({ "display": "dbm" })),
        ];
        system.right = vec![
            entry("s8", "swap-meter", serde_json::json!({})),
            entry(
                "s9",
                "swap-graph",
                serde_json::json!({ "display": "percent" }),
            ),
            entry("s10", "disk-meter", serde_json::json!({ "drive": "C:\\" })),
            entry(
                "s11",
                "disk-meter",
                serde_json::json!({ "drive": "Q:\\", "mode": "freeBytes" }),
            ),
            entry("s12", "recycle-bin", serde_json::json!({})),
        ];
        // Launchers (SPEC §9.17, §9.18, §9.21): Shortcuts to a program, a folder, a web
        // address, a tinted program, a bare program name and a missing target (inactive); the
        // system drive and an absent one (inactive); a Folder Menu, the Start Menu and Eject
        // USB; every Power mode, the last at 50 %.
        let launchers = Layout {
            left: vec![
                entry(
                    "l1",
                    "shortcut",
                    serde_json::json!({ "target": "C:\\Windows\\System32\\notepad.exe" }),
                ),
                entry(
                    "l2",
                    "shortcut",
                    serde_json::json!({ "target": "C:\\Windows" }),
                ),
                entry(
                    "l3",
                    "shortcut",
                    serde_json::json!({ "url": "https://example.com/" }),
                ),
                entry(
                    "l4",
                    "shortcut",
                    serde_json::json!({
                        "target": "C:\\Windows\\explorer.exe",
                        "color": { "mode": "custom", "color": "#20B7FF" }
                    }),
                ),
                entry(
                    "l5",
                    "shortcut",
                    serde_json::json!({ "target": "notepad.exe" }),
                ),
                entry(
                    "l6",
                    "shortcut",
                    serde_json::json!({ "target": "Q:\\Gone\\tool.exe" }),
                ),
                entry("d1", "disk", serde_json::json!({ "drive": "C:\\" })),
                entry("d2", "disk", serde_json::json!({ "drive": "Q:\\" })),
                entry(
                    "f1",
                    "folder-menu",
                    serde_json::json!({ "folder": "C:\\Windows" }),
                ),
                entry("m1", "start-menu", serde_json::json!({})),
                entry("u1", "eject-usb", serde_json::json!({})),
                entry("p1", "power", serde_json::json!({ "mode": "lock" })),
                entry("p2", "power", serde_json::json!({ "mode": "signOut" })),
                entry("p3", "power", serde_json::json!({ "mode": "switchUser" })),
                entry("p4", "power", serde_json::json!({})),
                entry("p5", "power", serde_json::json!({ "mode": "hibernate" })),
                entry("p6", "power", serde_json::json!({ "mode": "shutDown" })),
                entry(
                    "p7",
                    "power",
                    serde_json::json!({ "mode": "restart", "opacity": 50 }),
                ),
            ],
            ..Layout::default()
        };
        let platform = crate::platform::current();
        let first_run = crate::config::Config::first_run("1.0.0").layout;
        for (name, layout, settings, os, bar_width) in [
            (
                "bar-first-run",
                &first_run,
                Settings::default(),
                dark,
                700.0,
            ),
            (
                "bar-first-run-classic",
                &first_run,
                classic.clone(),
                dark,
                700.0,
            ),
            ("bar-general", &general, Settings::default(), dark, 700.0),
            (
                "bar-general-light",
                &general,
                Settings::default(),
                light,
                700.0,
            ),
            ("bar-system", &system, Settings::default(), dark, 1400.0),
            (
                "bar-system-light",
                &system,
                Settings::default(),
                light,
                1400.0,
            ),
            (
                "bar-launchers",
                &launchers,
                Settings::default(),
                dark,
                680.0,
            ),
            (
                "bar-launchers-light",
                &launchers,
                Settings::default(),
                light,
                680.0,
            ),
        ] {
            let theme = Theme::resolve(&settings, os);
            for (suffix, scale) in [("", 1.0), ("@1.5x", 1.5)] {
                let (size, width, height) = window(bar_width, theme.bar_height, scale);
                let mut sensors = BarSensors::default();
                let caps = crate::platform::Capabilities {
                    battery: true,
                    wifi: true,
                    ..platform.capabilities()
                };
                sensors.sync(
                    layout,
                    &Default::default(),
                    &caps,
                    crate::sensors::registry::registry(),
                );
                let locale = platform.locale(settings.date_locale);
                let numbers = crate::sensors::format::Numbers::from(&locale);
                let env = crate::bar::sensors::Env {
                    settings: &settings,
                    platform: platform.as_ref(),
                    alt: false,
                    animate: false,
                    locale: &locale,
                    numbers,
                    changes: &crate::sampler::ChangeSink::NONE,
                };
                // The clocks read the time at their first wake-up; the others get samples.
                sensors.wake(std::time::Instant::now(), &env);
                samples::feed(&mut sensors, &env);
                let cx = LayoutCtx {
                    theme: &theme,
                    text: text.as_ref(),
                    scale,
                    locale: &locale,
                    numbers,
                    icons: &crate::sensors::DirectIcons(platform.as_ref()),
                };
                sensors.update_layout(&cx, size.width);
                let skin = skin_of(&theme, size, scale, Edge::Bottom);
                let bitmap = render_offscreen(width, height, scale, &mut |canvas| {
                    let skin = skin.as_ref().map(|s| &s.bar);
                    paint_bar(canvas, size, &theme, Edge::Bottom, 0.0, skin);
                    sensors.draw(canvas, &theme, size.height);
                })
                .expect("rendered");
                save(&format!("{name}{suffix}"), &bitmap);
            }
        }

        // Hints (SPEC §7.1): a two-column hint above a bottom bar, one below a top bar, and
        // Date's calendar (a rich hint, §9.4).
        use crate::bar::hint::Prepared;
        use crate::bar::placement::Anchor;
        use crate::platform::time::Date;
        use crate::platform::{Locale, RectF};
        use crate::sensors::HintContent;
        use crate::sensors::date::MonthCalendar;
        for (name, settings, os, edge, calendar) in [
            (
                "hint-modern",
                Settings::default(),
                dark,
                Edge::Bottom,
                false,
            ),
            (
                "hint-modern-light-top",
                Settings::default(),
                light,
                Edge::Top,
                false,
            ),
            ("hint-classic", classic.clone(), dark, Edge::Bottom, false),
            (
                "hint-calendar",
                Settings::default(),
                dark,
                Edge::Bottom,
                true,
            ),
        ] {
            let theme = Theme::resolve(&settings, os);
            let bar = match edge {
                Edge::Bottom => RectF::new(0.0, 572.0, 800.0, 600.0),
                Edge::Top => RectF::new(0.0, 0.0, 800.0, 28.0),
            };
            let anchor = Anchor {
                rect: RectF::new(380.0, bar.top, 420.0, bar.bottom),
                bar,
                monitor: RectF::new(0.0, 0.0, 800.0, 600.0),
                edge,
            };
            let content = if calendar {
                let today = Date::new(2026, 10, 13).expect("a date");
                let names = Locale::en_us().weekdays_short;
                HintContent::Rich(Box::new(MonthCalendar::new(today, 0, names)))
            } else {
                let text = "Processor: 37%\nUser:\t25%\nKernel:\t12%\nCores:\t8 (16 threads)";
                HintContent::Text(text.into())
            };
            let hint = Prepared::new(content, text.as_ref(), &anchor, &theme).expect("not empty");
            for (suffix, scale) in [("", 1.0), ("@1.5x", 1.5)] {
                let width = (hint.frame.window.width() * scale).round() as u32;
                let height = (hint.frame.window.height() * scale).round() as u32;
                let bitmap = render_offscreen(width, height, scale, &mut |canvas| {
                    hint.paint(canvas, &theme)
                })
                .expect("rendered");
                save(&format!("{name}{suffix}"), &bitmap);
            }
        }

        // The Folder Menu's list (SPEC §9.19): C:\Windows with this machine's shell icons,
        // its left edge at the sensor's, the third row hovered.
        use crate::bar::popup::{self, Open};
        use crate::platform::{KnownFolder, PopupInput, UiWaker};
        use crate::sensors::common::folder_browser::{FolderBrowser, Options};
        use crate::sensors::popup::PopupCtx;
        let platform = crate::platform::current();
        let icons = crate::sensors::DirectIcons(platform.as_ref());
        let waker = UiWaker::new(|| {});
        // The Start Menu's list merges the user's and all users' programs (SPEC §9.20).
        let start_menu: Vec<String> = [KnownFolder::Programs, KnownFolder::CommonPrograms]
            .into_iter()
            .filter_map(|f| platform.known_folder(f))
            .collect();
        let browser = |folders: &[String], title: &str| -> Box<dyn crate::sensors::popup::Popup> {
            Box::new(FolderBrowser::new(Options {
                folders: folders.to_vec(),
                title: title.into(),
                show_hidden: false,
                width: 240.0,
                height: 320.0,
            }))
        };
        let windows = ["C:\\Windows".to_owned()];
        // Eject USB's list (SPEC §9.22): a stick, a USB disk and a disc, the first button
        // hovered.
        let eject = || -> Box<dyn crate::sensors::popup::Popup> {
            use crate::platform::{DriveKind, Volume};
            use crate::sensors::common::eject_list::{Drives, EjectList};
            let mut disk = Volume::new("F:\\", DriveKind::Fixed, "Backup");
            disk.ejectable = true;
            let volumes = vec![
                Volume::new("E:\\", DriveKind::Removable, "STICK"),
                disk,
                Volume::new("D:\\", DriveKind::Optical, "HOLIDAY_2026"),
            ];
            let drives = std::sync::Arc::new(std::sync::Mutex::new(Drives {
                volumes,
                generation: 1,
            }));
            let mut list = EjectList::new(drives, crate::sampler::ChangeSink::NONE);
            list.preset_sizes(&[
                ("E:\\", 15_931_539_456),
                ("F:\\", 2_000_381_014_016),
                ("D:\\", 4_700_372_992),
            ]);
            Box::new(list)
        };
        type MakePopup<'a> = Box<dyn Fn() -> Box<dyn crate::sensors::popup::Popup> + 'a>;
        let popups: [(&str, _, MakePopup, _); 4] = [
            (
                "popup-folder",
                dark,
                Box::new(|| browser(&windows, "Windows")),
                (60.0, 75.0),
            ),
            (
                "popup-folder-light",
                light,
                Box::new(|| browser(&windows, "Windows")),
                (60.0, 75.0),
            ),
            (
                "popup-start-menu",
                dark,
                Box::new(|| browser(&start_menu, "Start Menu")),
                (60.0, 75.0),
            ),
            ("popup-eject", dark, Box::new(eject), (270.0, 14.0)),
        ];
        for (name, os, make, (hover_x, hover_y)) in popups {
            let settings = Settings::default();
            let theme = Theme::resolve(&settings, os);
            let bar = RectF::new(0.0, 572.0, 800.0, 600.0);
            let anchor = Anchor {
                rect: RectF::new(300.0, bar.top, 321.0, bar.bottom),
                bar,
                monitor: RectF::new(0.0, 0.0, 800.0, 600.0),
                edge: Edge::Bottom,
            };
            for (suffix, scale) in [("", 1.0), ("@1.5x", 1.5)] {
                let mut open = Open {
                    owner: "f".into(),
                    popup: make(),
                    frame: None,
                    room: popup::room(&anchor),
                    monitor_height: anchor.monitor.height(),
                    free_left: false,
                    captured: false,
                };
                let cx = PopupCtx {
                    theme: &theme,
                    text: text.as_ref(),
                    scale,
                    platform: &platform,
                    icons: &icons,
                    waker: &waker,
                    room: open.room,
                    monitor_height: open.monitor_height,
                    edge: Edge::Bottom,
                    free_left: false,
                    animate: false,
                    numbers: Default::default(),
                    now: std::time::Instant::now(),
                };
                open.popup.layout(&cx);
                // A listing slower than the first wait arrives meanwhile.
                std::thread::sleep(std::time::Duration::from_millis(300));
                open.popup.poll(&cx);
                open.popup.layout(&cx);
                let hover = PopupInput::PointerMove(crate::platform::PointF::new(hover_x, hover_y));
                open.popup.input(&hover, &cx);
                let size = open.popup.layout(&cx);
                open.place(size, &anchor, &theme);
                let frame = open.frame.expect("placed");
                let width = (frame.window.width() * scale).round() as u32;
                let height = (frame.window.height() * scale).round() as u32;
                let bitmap = render_offscreen(width, height, scale, &mut |canvas| {
                    popup::paint(canvas, &open, &theme)
                })
                .expect("rendered");
                save(&format!("{name}{suffix}"), &bitmap);
            }
        }
    }
}

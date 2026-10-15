//! Drawing the bar (SPEC §5.5, §12). Sensors arrive in M1 S4; until then the bar is its
//! background and hairline.

use super::theme::Theme;
use crate::config::model::Edge;
use crate::platform::{Canvas, RectF, SizeF};

/// The hover highlight multiplies the background opacity by up to this (SPEC §5.5).
const HIGHLIGHT_GAIN: f32 = 1.7;

/// Draws the bar's background into a cleared canvas of `size` logical px; `highlight` (0–1) is
/// how far the hover highlight has faded in.
pub(super) fn paint_bar(
    canvas: &mut dyn Canvas,
    size: SizeF,
    theme: &Theme,
    edge: Edge,
    highlight: f32,
) {
    let background = theme.bar_background;
    let gain = 1.0 + (HIGHLIGHT_GAIN - 1.0) * highlight;
    canvas.fill_rect(
        RectF::from_size(size),
        background.with_alpha(background.a * gain),
    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::{Settings, Theme as ThemeId};
    use crate::platform::recording::RecordingCanvas;
    use crate::platform::{Color, OsAppearance};

    #[test]
    fn modern_bar_has_a_hairline_towards_the_screen_centre() {
        let theme = Theme::resolve(&Settings::default(), OsAppearance::default());
        let size = SizeF::new(800.0, 28.0);

        let mut bottom = RecordingCanvas::new(1.0);
        paint_bar(&mut bottom, size, &theme, Edge::Bottom, 0.0);
        assert_eq!(
            bottom.dump(),
            "fill rect 0,0 800×28 #1C1C1F/80%\nfill rect 0,0 800×1 #FFFFFF/8%\n"
        );

        // At 150 % the line is two physical px, so it stays crisp.
        let mut top = RecordingCanvas::new(1.5);
        paint_bar(&mut top, size, &theme, Edge::Top, 0.0);
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
        );
        assert!(
            canvas
                .dump()
                .starts_with("fill rect 0,0 100×28 #1C1C1F/54%\n")
        );
    }

    #[test]
    fn classic_bar_has_no_hairline() {
        let s = Settings {
            theme: ThemeId::Classic,
            ..Settings::default()
        };
        let theme = Theme::resolve(&s, OsAppearance::default());
        let mut canvas = RecordingCanvas::new(1.0);
        paint_bar(
            &mut canvas,
            SizeF::new(100.0, 25.0),
            &theme,
            Edge::Bottom,
            0.0,
        );
        assert_eq!(canvas.ops.len(), 1);
        assert_eq!(theme.bar_background, Color::rgba(0x2A, 0x2A, 0x2E, 0.9));
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

    /// A sample of every canvas operation in bar-sized pieces, for reviewing the renderer
    /// (CI screenshots) until the sensors draw the bar.
    pub(super) mod specimen {
        use std::f32::consts::TAU;
        use std::path::Path;

        use crate::bar::theme::{Inactive, Theme};
        use crate::platform::{
            Bitmap, Brush, Canvas, Color, GradientStop, PointF, RectF, Shape, SizeF, TextEngine,
            TextStyle, load_image,
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
            let stop = |offset, color| GradientStop { offset, color };
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
            super::super::paint_bar(
                canvas,
                SizeF::new(WIDTH, h),
                theme,
                crate::config::model::Edge::Bottom,
                0.0,
            );
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
                Topic::Uptime => Sample::Uptime(UptimeSample {
                    boot_unix_ms: 0,
                    uptime_secs: 3 * 86_400 + 4 * 3600 + 12 * 60,
                    read_at: std::time::Instant::now(),
                }),
            }
        }

        pub fn feed(sensors: &mut BarSensors, env: &Env) {
            let mut topics: Vec<Topic> = Vec::new();
            for request in sensors.subscriptions().into_values().flatten() {
                if !topics.contains(&request.topic) {
                    topics.push(request.topic);
                }
            }
            // One sample a second, as the sampler would read them.
            let start = std::time::Instant::now();
            for i in 0..150 {
                let now = start + std::time::Duration::from_secs(i as u64);
                for topic in &topics {
                    let delivery = Delivery {
                        topic: topic.clone(),
                        sample: Arc::new(sample(topic, i)),
                        to: None,
                    };
                    sensors.deliver(&delivery, env, now);
                }
            }
        }
    }

    /// Renders the bar with the real renderer to `target/screenshots/bar-*.png` (CI uploads
    /// them): plain bars per theme, the canvas specimen, the first-run layout, the General
    /// sensors, every system sensor with made-up samples and the launchers, at 100 % and
    /// 150 %.
    #[cfg(windows)]
    #[test]
    #[ignore = "writes PNGs to target/screenshots; run with --ignored"]
    fn render_bar() {
        use crate::bar::sensors::BarSensors;
        use crate::config::model::{Layout, SensorInstance};
        use crate::platform::{render_offscreen, text_engine};
        use crate::sensors::LayoutCtx;
        use crate::test_util::{screenshot_path, write_png};

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
        let bars = [
            ("bar-modern-dark", Settings::default(), dark, Edge::Bottom),
            (
                "bar-modern-light-top",
                Settings::default(),
                light,
                Edge::Top,
            ),
            ("bar-classic", classic.clone(), dark, Edge::Bottom),
        ];
        for (name, settings, os, edge) in bars {
            let theme = Theme::resolve(&settings, os);
            let size = SizeF::new(400.0, theme.bar_height as f32);
            let bitmap = render_offscreen(400, theme.bar_height, 1.0, &mut |canvas| {
                paint_bar(canvas, size, &theme, edge, 0.0)
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
        // network and uptime in the centre, swap and disks on the right (Q:\ is absent).
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
        ];
        // Launchers (SPEC §9.17): a program, a folder, a web address, a tinted program, a
        // bare program name and a missing target (inactive).
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
            ("bar-system", &system, Settings::default(), dark, 1100.0),
            (
                "bar-system-light",
                &system,
                Settings::default(),
                light,
                1100.0,
            ),
            (
                "bar-launchers",
                &launchers,
                Settings::default(),
                dark,
                300.0,
            ),
            (
                "bar-launchers-light",
                &launchers,
                Settings::default(),
                light,
                300.0,
            ),
        ] {
            let theme = Theme::resolve(&settings, os);
            for (suffix, scale) in [("", 1.0), ("@1.5x", 1.5)] {
                let size = SizeF::new(bar_width, theme.bar_height as f32);
                let mut sensors = BarSensors::default();
                sensors.sync(
                    layout,
                    &Default::default(),
                    &platform.capabilities(),
                    crate::sensors::registry::registry(),
                );
                let env = crate::bar::sensors::Env {
                    settings: &settings,
                    theme: &theme,
                    platform: platform.as_ref(),
                    alt: false,
                    animate: false,
                };
                // The clocks read the time at their first wake-up; the others get samples.
                sensors.wake(std::time::Instant::now(), &env);
                samples::feed(&mut sensors, &env);
                let cx = LayoutCtx {
                    theme: &theme,
                    text: text.as_ref(),
                    scale,
                    numbers: platform.locale(settings.date_locale).numbers(),
                    icons: &crate::sensors::DirectIcons(platform.as_ref()),
                };
                sensors.update_layout(&cx, size.width);
                let width = (size.width * scale).round() as u32;
                let height = (size.height * scale).round() as u32;
                let bitmap = render_offscreen(width, height, scale, &mut |canvas| {
                    paint_bar(canvas, size, &theme, Edge::Bottom, 0.0);
                    sensors.draw(canvas, &theme, size.height);
                })
                .expect("rendered");
                save(&format!("{name}{suffix}"), &bitmap);
            }
        }

        // Hints (SPEC §7.1): a two-column hint above a bottom bar, one below a top bar, and
        // Date's calendar (a rich hint, §9.4).
        use crate::bar::hint::{self, HintBody, HintFrame};
        use crate::bar::placement::Anchor;
        use crate::platform::{Locale, RectF};
        use crate::sensors::HintContent;
        use crate::sensors::date::MonthCalendar;
        use crate::sensors::dates::Date;
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
            let body = HintBody::new(content, text.as_ref(), &theme).expect("not empty");
            let frame = HintFrame::new(body.size(), &anchor, &theme);
            for (suffix, scale) in [("", 1.0), ("@1.5x", 1.5)] {
                let width = (frame.window.width() * scale).round() as u32;
                let height = (frame.window.height() * scale).round() as u32;
                let bitmap = render_offscreen(width, height, scale, &mut |canvas| {
                    hint::paint(canvas, &frame, &body, &theme)
                })
                .expect("rendered");
                save(&format!("{name}{suffix}"), &bitmap);
            }
        }
    }
}

//! Skin packs (SPEC §12.4): a folder in the skins folder whose `skin.json` names its tiles.
//! A pack is checked when listed and again when drawn: only PNG and JPEG files named in
//! `skin.json`, each at most 1 MB and 512 px a side, as tall as the bar part and the shadow band
//! it declares (± 1 px).

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::{BuiltIn, CUSTOM};
use crate::config::model::Edge;
use crate::i18n;
use crate::platform::bitmap::ImageFormat;

/// The file every pack has.
pub const MANIFEST: &str = "skin.json";
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
pub(super) const MAX_IMAGE_BYTES: u64 = 1024 * 1024;
const MAX_SIDE: u32 = 512;
/// The bar height tiles are drawn for unless `skin.json` says otherwise.
const DEFAULT_HEIGHT: u32 = 25;

fn default_height() -> u32 {
    DEFAULT_HEIGHT
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    #[serde(default)]
    name: String,
    image: Option<String>,
    image_top: Option<String>,
    image_bottom: Option<String>,
    #[serde(default = "default_height")]
    height: u32,
    #[serde(default)]
    shadow_band: u32,
}

/// A pack that passed the checks.
#[derive(Debug, Clone, PartialEq)]
pub struct Pack {
    /// Its folder's name, which `classic.skin` stores.
    pub id: String,
    /// `skin.json`'s name, or the id.
    pub name: String,
    pub dir: PathBuf,
    image: Option<String>,
    image_top: Option<String>,
    image_bottom: Option<String>,
    /// The bar height the tiles were drawn for, px.
    pub height: u32,
    /// The transparent shadow on the side facing the screen centre, px; 0 for none.
    pub shadow_band: u32,
}

/// How a tile is cut: whether its shadow band comes first (on top), and whether the band is
/// mirrored to face the other way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileLayout {
    pub band_first: bool,
    pub mirror_band: bool,
}

impl Pack {
    /// The file name of the tile for a bar on `edge`, and how to cut it. A tile has its band on
    /// the side facing the screen centre for the edge it is drawn for: `imageBottom` and `image`
    /// on top, `imageTop` at the bottom; `image` on a top bar has its band mirrored below it.
    pub fn tile(&self, edge: Edge) -> (String, TileLayout) {
        let shared = || self.image.clone().unwrap_or_default();
        let (name, layout) = match (edge, &self.image_top) {
            (Edge::Top, Some(top)) => (
                top.clone(),
                TileLayout {
                    band_first: false,
                    mirror_band: false,
                },
            ),
            (Edge::Top, None) => (
                shared(),
                TileLayout {
                    band_first: true,
                    mirror_band: true,
                },
            ),
            (Edge::Bottom, _) => (
                self.image_bottom.clone().unwrap_or_else(shared),
                TileLayout {
                    band_first: true,
                    mirror_band: false,
                },
            ),
        };
        (name, layout)
    }

    /// A tile's height in px: the bar part and the band.
    pub fn tile_height(&self) -> u32 {
        self.height + self.shadow_band
    }

    /// The files `skin.json` names.
    pub fn files(&self) -> Vec<&str> {
        [&self.image, &self.image_top, &self.image_bottom]
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect()
    }
}

/// Why a pack, or the custom image, cannot be used. Settings shows [`PackError::message`].
#[derive(Debug, Clone, PartialEq)]
pub enum PackError {
    /// `skin.json` is too large or not valid.
    Manifest(String),
    /// The folder has a built-in skin's name.
    Reserved,
    /// `skin.json` names no tile for one of the edges.
    NoImage,
    /// The height and shadow band together are not 1–512 px.
    Height,
    /// A name in `skin.json` that is not a file name in the pack's folder.
    FileName(String),
    Missing(String),
    TooLarge(String),
    /// Not a PNG or JPEG image.
    Format(String),
    /// Wider or taller than 512 px.
    Dimensions(String),
    /// A tile's height does not match the bar part and band (± 1 px).
    TileHeight {
        file: String,
        actual: u32,
        expected: u32,
    },
}

impl PackError {
    pub fn message(&self) -> String {
        let file = |key: &str, file: &str| i18n::t_args(key, &[("file", file)]);
        match self {
            Self::Manifest(error) => i18n::t_args("skins.error.manifest", &[("error", error)]),
            Self::Reserved => i18n::t("skins.error.reserved"),
            Self::NoImage => i18n::t("skins.error.noImage"),
            Self::Height => i18n::t("skins.error.height"),
            Self::FileName(f) => file("skins.error.fileName", f),
            Self::Missing(f) => file("skins.error.missing", f),
            Self::TooLarge(f) => file("skins.error.tooLarge", f),
            Self::Format(f) => file("skins.error.format", f),
            Self::Dimensions(f) => file("skins.error.dimensions", f),
            Self::TileHeight {
                file,
                actual,
                expected,
            } => i18n::t_args(
                "skins.error.tileHeight",
                &[
                    ("file", file),
                    ("actual", &actual.to_string()),
                    ("expected", &expected.to_string()),
                ],
            ),
        }
    }
}

/// A folder in the skins folder, with its pack or why it cannot be used.
#[derive(Debug, Clone, PartialEq)]
pub struct PackEntry {
    pub id: String,
    pub pack: Result<Pack, PackError>,
}

/// Every folder in `dir` as a pack, sorted by name; none when `dir` does not exist. Folders
/// being installed or removed are left out.
pub fn list(dir: &Path) -> Vec<PackEntry> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut packs: Vec<PackEntry> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|id| !id.starts_with(super::install::STAGING_PREFIX))
        .map(|id| PackEntry {
            pack: read(&id, &dir.join(&id)),
            id,
        })
        .collect();
    packs.sort_by_key(|p| p.id.to_lowercase());
    packs
}

/// The pack `id` in `dir`; `None` when there is no such folder.
pub fn load(dir: &Path, id: &str) -> Option<Result<Pack, PackError>> {
    let folder = dir.join(id);
    (is_file_name(id) && folder.is_dir()).then(|| read(id, &folder))
}

pub(super) fn read(id: &str, folder: &Path) -> Result<Pack, PackError> {
    if BuiltIn::from_id(id).is_some() || id == CUSTOM {
        return Err(PackError::Reserved);
    }
    let path = folder.join(MANIFEST);
    let size = fs::metadata(&path)
        .ok()
        .filter(|m| m.is_file())
        .ok_or_else(|| PackError::Missing(MANIFEST.into()))?
        .len();
    if size > MAX_MANIFEST_BYTES {
        return Err(PackError::TooLarge(MANIFEST.into()));
    }
    let text = fs::read(&path).map_err(|_| PackError::Missing(MANIFEST.into()))?;
    let manifest: Manifest =
        serde_json::from_slice(&text).map_err(|e| PackError::Manifest(e.to_string()))?;
    let tile_height = manifest
        .height
        .checked_add(manifest.shadow_band)
        .filter(|h| manifest.height > 0 && *h <= MAX_SIDE)
        .ok_or(PackError::Height)?;
    let has_tile = |own: &Option<String>| own.is_some() || manifest.image.is_some();
    if !has_tile(&manifest.image_top) || !has_tile(&manifest.image_bottom) {
        return Err(PackError::NoImage);
    }
    let named = [&manifest.image, &manifest.image_top, &manifest.image_bottom];
    for name in named.into_iter().flatten() {
        check_file(folder, name, tile_height, manifest.shadow_band)?;
    }
    Ok(Pack {
        id: id.into(),
        name: if manifest.name.trim().is_empty() {
            id.into()
        } else {
            manifest.name.trim().into()
        },
        dir: folder.to_path_buf(),
        image: manifest.image,
        image_top: manifest.image_top,
        image_bottom: manifest.image_bottom,
        height: manifest.height,
        shadow_band: manifest.shadow_band,
    })
}

/// A plain name of something in a folder: no path, no drive, not `.` or `..`.
pub(super) fn is_file_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':'])
        && Path::new(name).file_name() == Some(std::ffi::OsStr::new(name))
}

/// Reads the tile `name` in `folder` after checking it (see [`check_image`]).
pub fn check_file(
    folder: &Path,
    name: &str,
    tile_height: u32,
    band: u32,
) -> Result<Vec<u8>, PackError> {
    if !is_file_name(name) {
        return Err(PackError::FileName(name.into()));
    }
    let bytes = read_image(&folder.join(name), name)?;
    check_image(name, &bytes, Some((tile_height, band)))?;
    Ok(bytes)
}

/// Reads an image file of at most 1 MB; `name` is what errors call it.
pub fn read_image(path: &Path, name: &str) -> Result<Vec<u8>, PackError> {
    let size = fs::metadata(path)
        .ok()
        .filter(|m| m.is_file())
        .ok_or_else(|| PackError::Missing(name.into()))?
        .len();
    if size > MAX_IMAGE_BYTES {
        return Err(PackError::TooLarge(name.into()));
    }
    fs::read(path).map_err(|_| PackError::Missing(name.into()))
}

/// A PNG or JPEG image at most 512 px a side; with `tile`, `(height, band)`, it must also be
/// `height` px tall (± 1 px) and taller than `band`.
pub fn check_image(name: &str, bytes: &[u8], tile: Option<(u32, u32)>) -> Result<(), PackError> {
    if !matches!(
        ImageFormat::sniff(bytes),
        Some(ImageFormat::Png | ImageFormat::Jpeg)
    ) {
        return Err(PackError::Format(name.into()));
    }
    let size = imagesize::blob_size(bytes).map_err(|_| PackError::Format(name.into()))?;
    let (width, height) = (
        u32::try_from(size.width).unwrap_or(u32::MAX),
        u32::try_from(size.height).unwrap_or(u32::MAX),
    );
    if width == 0 || height == 0 {
        return Err(PackError::Format(name.into()));
    }
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err(PackError::Dimensions(name.into()));
    }
    if let Some((expected, band)) = tile
        && (height.abs_diff(expected) > 1 || height <= band)
    {
        return Err(PackError::TileHeight {
            file: name.into(),
            actual: height,
            expected,
        });
    }
    Ok(())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::test_util::write_png;

    /// Writes an opaque `width` × `height` PNG of one colour.
    pub fn png(path: &Path, width: u32, height: u32, rgb: [u8; 3]) {
        let rgba = [rgb[0], rgb[1], rgb[2], 255].repeat((width * height) as usize);
        write_png(path, width, height, &rgba);
    }

    /// A pack folder `id` in `dir` with this `skin.json`; returns the folder.
    pub fn pack(dir: &Path, id: &str, manifest: &str) -> PathBuf {
        let folder = dir.join(id);
        fs::create_dir_all(&folder).expect("folder");
        fs::write(folder.join(MANIFEST), manifest).expect("manifest");
        folder
    }

    /// The start of a JPEG, as far as its size: SOI, then SOF0 with the height and width.
    fn jpeg_header(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08];
        bytes.extend(height.to_be_bytes());
        bytes.extend(width.to_be_bytes());
        bytes.extend([3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        bytes
    }

    #[test]
    fn reads_a_pack() {
        let dir = tempfile::tempdir().expect("temp dir");
        let folder = pack(
            dir.path(),
            "blue",
            r#"{ "name": "Blue Steel", "image": "tile.png", "imageTop": "top.png",
                 "height": 25, "shadowBand": 9 }"#,
        );
        png(&folder.join("tile.png"), 16, 34, [0, 0, 255]);
        png(&folder.join("top.png"), 16, 33, [0, 0, 200]);
        let pack = load(dir.path(), "blue")
            .expect("folder exists")
            .expect("valid");
        assert_eq!(pack.name, "Blue Steel");
        assert_eq!(
            (pack.height, pack.shadow_band, pack.tile_height()),
            (25, 9, 34)
        );
        // Bottom: the shared image, band on top. Top: its own image, band at the bottom.
        let (bottom, layout) = pack.tile(Edge::Bottom);
        assert_eq!(bottom, "tile.png");
        assert!(layout.band_first && !layout.mirror_band);
        let (top, layout) = pack.tile(Edge::Top);
        assert_eq!(top, "top.png");
        assert!(!layout.band_first && !layout.mirror_band);
    }

    #[test]
    fn shared_image_is_mirrored_on_a_top_bar() {
        let dir = tempfile::tempdir().expect("temp dir");
        let folder = pack(dir.path(), "plain", r#"{ "image": "tile.png" }"#);
        png(&folder.join("tile.png"), 8, 25, [40, 40, 40]);
        let pack = load(dir.path(), "plain").expect("exists").expect("valid");
        // Defaults: the id as name, 25 px, no band.
        assert_eq!(pack.name, "plain");
        assert_eq!((pack.height, pack.shadow_band), (25, 0));
        let (name, layout) = pack.tile(Edge::Top);
        assert_eq!(name, "tile.png");
        assert!(layout.band_first && layout.mirror_band);
    }

    #[test]
    fn checks_every_named_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let d = dir.path();
        let check = |id: &str, manifest: &str, files: &[(&str, u32)]| {
            let folder = pack(d, id, manifest);
            for (name, height) in files {
                png(&folder.join(name), 4, *height, [9, 9, 9]);
            }
            load(d, id).expect("exists").err()
        };
        assert_eq!(
            check("ok", r#"{ "image": "t.png" }"#, &[("t.png", 26)]),
            None
        );
        assert_eq!(
            check("none", r#"{ "name": "x" }"#, &[]),
            Some(PackError::NoImage)
        );
        // A top tile alone leaves the bottom without one.
        assert_eq!(
            check("half", r#"{ "imageTop": "t.png" }"#, &[("t.png", 25)]),
            Some(PackError::NoImage)
        );
        assert_eq!(
            check(
                "both",
                r#"{ "imageTop": "t.png", "imageBottom": "b.png" }"#,
                &[("t.png", 25), ("b.png", 24)]
            ),
            None
        );
        assert_eq!(
            check("gone", r#"{ "image": "t.png" }"#, &[]),
            Some(PackError::Missing("t.png".into()))
        );
        assert_eq!(
            check("up", r#"{ "image": "../ok/t.png" }"#, &[]),
            Some(PackError::FileName("../ok/t.png".into()))
        );
        assert_eq!(
            check("abs", r#"{ "image": "C:\\Windows\\a.png" }"#, &[]),
            Some(PackError::FileName("C:\\Windows\\a.png".into()))
        );
        assert_eq!(
            check(
                "short",
                r#"{ "image": "t.png", "shadowBand": 9 }"#,
                &[("t.png", 25)]
            ),
            Some(PackError::TileHeight {
                file: "t.png".into(),
                actual: 25,
                expected: 34
            })
        );
        assert_eq!(
            check("tall", r#"{ "image": "t.png", "height": 600 }"#, &[]),
            Some(PackError::Height)
        );
        assert_eq!(
            check("zero", r#"{ "image": "t.png", "height": 0 }"#, &[]),
            Some(PackError::Height)
        );
        assert!(matches!(
            check("json", r#"{ "image": "#, &[]),
            Some(PackError::Manifest(_))
        ));
        assert_eq!(
            check("gloss", r#"{ "image": "t.png" }"#, &[("t.png", 25)]),
            Some(PackError::Reserved)
        );
        let folder = d.join("bare");
        fs::create_dir_all(&folder).expect("folder");
        assert_eq!(
            load(d, "bare").expect("exists").err(),
            Some(PackError::Missing(MANIFEST.into()))
        );
        assert_eq!(load(d, "nowhere"), None);
        assert_eq!(load(d, ".."), None);
    }

    #[test]
    fn checks_images() {
        let png_bytes = |w: u32, h: u32| {
            let dir = tempfile::tempdir().expect("temp dir");
            let path = dir.path().join("x.png");
            png(&path, w, h, [1, 2, 3]);
            fs::read(path).expect("written")
        };
        assert_eq!(check_image("x", &png_bytes(512, 512), None), Ok(()));
        assert_eq!(
            check_image("x", &png_bytes(513, 20), None),
            Err(PackError::Dimensions("x".into()))
        );
        // A JPEG is checked by its header.
        assert_eq!(
            check_image("j", &jpeg_header(64, 25), Some((25, 0))),
            Ok(())
        );
        assert_eq!(
            check_image("j", &jpeg_header(64, 600), None),
            Err(PackError::Dimensions("j".into()))
        );
        // Other formats, also by their contents rather than their names.
        assert_eq!(
            check_image("g.png", b"GIF89a\x01\x00\x01\x00", None),
            Err(PackError::Format("g.png".into()))
        );
        assert_eq!(
            check_image("t.png", b"\x89PNG\r\n\x1a\n", None),
            Err(PackError::Format("t.png".into()))
        );
        // A tile no taller than its band has no bar part.
        assert_eq!(
            check_image("t", &png_bytes(4, 9), Some((10, 9))),
            Err(PackError::TileHeight {
                file: "t".into(),
                actual: 9,
                expected: 10
            })
        );
    }

    #[test]
    fn files_over_1_mb_are_refused() {
        let dir = tempfile::tempdir().expect("temp dir");
        let folder = pack(dir.path(), "big", r#"{ "image": "t.png" }"#);
        fs::write(folder.join("t.png"), vec![0u8; 1024 * 1024 + 1]).expect("written");
        assert_eq!(
            load(dir.path(), "big").expect("exists").err(),
            Some(PackError::TooLarge("t.png".into()))
        );
    }

    #[test]
    fn lists_packs_by_name_with_their_problems() {
        let dir = tempfile::tempdir().expect("temp dir");
        let d = dir.path();
        png(
            &pack(d, "Zebra", r#"{ "image": "t.png" }"#).join("t.png"),
            4,
            25,
            [1, 1, 1],
        );
        pack(d, "apple", r#"{ "image": "t.png" }"#);
        fs::write(d.join("loose.png"), b"not a folder").expect("written");
        let listed = list(d);
        let ids: Vec<&str> = listed.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["apple", "Zebra"]);
        assert_eq!(listed[0].pack, Err(PackError::Missing("t.png".into())));
        assert!(listed[1].pack.is_ok());
        assert!(list(&d.join("missing")).is_empty());
    }

    #[test]
    fn every_problem_has_a_message() {
        let file = || "t.png".to_owned();
        for e in [
            PackError::Manifest("bad".into()),
            PackError::Reserved,
            PackError::NoImage,
            PackError::Height,
            PackError::FileName(file()),
            PackError::Missing(file()),
            PackError::TooLarge(file()),
            PackError::Format(file()),
            PackError::Dimensions(file()),
            PackError::TileHeight {
                file: file(),
                actual: 20,
                expected: 34,
            },
        ] {
            let message = e.message();
            assert!(!message.starts_with("skins."), "{e:?}: {message}");
            assert!(!message.contains('{'), "{e:?}: {message}");
        }
    }
}

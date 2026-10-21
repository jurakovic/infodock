//! Installing skin packs from Settings (SPEC §12.4): a pack's folder or a `.zip` holding it is
//! copied into the skins folder. Only `skin.json` and the files it names are copied, and the copy
//! is checked as the bar checks a pack before it takes the place of the pack with that name.

use std::fs;
use std::path::{Path, PathBuf};

use super::pack::{self, MANIFEST, PackError};
use super::zip::{self, ZipError};
use crate::i18n::{self, t_args};

/// The largest `.zip` read.
const MAX_ZIP_BYTES: u64 = 32 * 1024 * 1024;
/// Folders in the skins folder starting with this are a pack being installed or removed; they
/// are not listed as packs.
pub const STAGING_PREFIX: char = '.';

/// Why a pack could not be installed. Settings shows [`InstallError::message`].
#[derive(Debug, Clone, PartialEq)]
pub enum InstallError {
    /// The folder, or the `.zip`, holds no `skin.json` (a `.zip` at its top or in one folder).
    NotAPack,
    /// The pack's name cannot be a folder in the skins folder.
    Name(String),
    Pack(PackError),
    Zip(ZipError),
    TooLarge,
    Io(String),
}

impl InstallError {
    pub fn message(&self) -> String {
        match self {
            Self::NotAPack => i18n::t("skins.install.notAPack"),
            Self::Name(name) => t_args("skins.install.name", &[("name", name)]),
            Self::Pack(e) => e.message(),
            Self::Zip(ZipError::Invalid) => i18n::t("skins.install.zipInvalid"),
            Self::Zip(ZipError::Unsupported) => i18n::t("skins.install.zipUnsupported"),
            Self::TooLarge => i18n::t("skins.install.tooLarge"),
            Self::Io(error) => t_args("skins.install.io", &[("error", error)]),
        }
    }
}

impl From<PackError> for InstallError {
    fn from(e: PackError) -> Self {
        Self::Pack(e)
    }
}

impl From<ZipError> for InstallError {
    fn from(e: ZipError) -> Self {
        Self::Zip(e)
    }
}

fn io(e: std::io::Error) -> InstallError {
    InstallError::Io(e.to_string())
}

/// Installs the pack in folder `source`, or in the `.zip` file `source`, into the skins folder
/// `dir`, replacing an installed pack of the same name; returns the pack as installed. A pack's
/// name is its folder's: in a `.zip`, the folder holding `skin.json`, or the file's name when
/// `skin.json` is at its top.
pub fn install(dir: &Path, source: &Path) -> Result<pack::Pack, InstallError> {
    if source.is_dir() {
        install_folder(dir, source)
    } else {
        install_zip(dir, source)
    }
}

fn install_folder(dir: &Path, source: &Path) -> Result<pack::Pack, InstallError> {
    let id = name_of(source.file_name().and_then(|n| n.to_str()))?;
    if !source.join(MANIFEST).is_file() {
        return Err(InstallError::NotAPack);
    }
    // Checked where it is, so only what it names is copied.
    let found = pack::read(&id, source)?;
    stage(dir, &id, |staging| {
        for name in found.files().into_iter().chain([MANIFEST]) {
            fs::copy(source.join(name), staging.join(name)).map_err(io)?;
        }
        Ok(())
    })
}

fn install_zip(dir: &Path, source: &Path) -> Result<pack::Pack, InstallError> {
    let size = fs::metadata(source).map_err(io)?.len();
    if size > MAX_ZIP_BYTES {
        return Err(InstallError::TooLarge);
    }
    let bytes = fs::read(source).map_err(io)?;
    let entries = zip::entries(&bytes)?;
    // `skin.json` at the top, or in one folder at the top.
    let folder = entries
        .iter()
        .filter_map(|e| e.name.strip_suffix(MANIFEST))
        .filter(|prefix| {
            prefix.is_empty() || (prefix.ends_with('/') && prefix.matches('/').count() == 1)
        })
        .min_by_key(|prefix| prefix.len())
        .ok_or(InstallError::NotAPack)?
        .to_owned();
    let id = match folder.strip_suffix('/') {
        Some(name) => name_of(Some(name))?,
        None => name_of(source.file_stem().and_then(|n| n.to_str()))?,
    };
    let entry = |name: &str| {
        let path = format!("{folder}{name}");
        entries.iter().find(|e| e.name.eq_ignore_ascii_case(&path))
    };
    // `skin.json` first, and only the files it names after it: nothing else in the archive is
    // extracted, so at most a few MB are written whatever it holds.
    let manifest = entry(MANIFEST).ok_or(InstallError::NotAPack)?;
    if manifest.size > pack::MAX_MANIFEST_BYTES {
        return Err(PackError::TooLarge(MANIFEST.into()).into());
    }
    let manifest = zip::extract(&bytes, manifest)?;
    let named = pack::named_files(&manifest)?;
    stage(dir, &id, |staging| {
        fs::write(staging.join(MANIFEST), &manifest).map_err(io)?;
        for name in &named {
            // A name that is not a plain file name, or a file not in the archive, fails the
            // pack's checks with its reason.
            let Some(entry) = entry(name).filter(|_| pack::is_file_name(name)) else {
                continue;
            };
            if entry.size > pack::MAX_IMAGE_BYTES {
                return Err(PackError::TooLarge(name.clone()).into());
            }
            fs::write(staging.join(name), zip::extract(&bytes, entry)?).map_err(io)?;
        }
        Ok(())
    })
}

/// A pack's name as a folder name in the skins folder.
fn name_of(name: Option<&str>) -> Result<String, InstallError> {
    let name = name.unwrap_or_default();
    if pack::is_file_name(name) && !name.starts_with(STAGING_PREFIX) {
        Ok(name.to_owned())
    } else {
        Err(InstallError::Name(name.to_owned()))
    }
}

/// Fills a staging folder in `dir` with `fill`, checks the pack there, keeps only what it names,
/// and puts it in place of the pack `id`.
fn stage(
    dir: &Path,
    id: &str,
    fill: impl FnOnce(&Path) -> Result<(), InstallError>,
) -> Result<pack::Pack, InstallError> {
    let staging = dir.join(format!("{STAGING_PREFIX}install-{id}"));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).map_err(io)?;
    let result = fill(&staging)
        .and_then(|()| pack::read(id, &staging).map_err(InstallError::from))
        .and_then(|staged| {
            keep_only(&staging, &staged)?;
            replace(dir, id, &staging)
        });
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result?;
    pack::read(id, &dir.join(id)).map_err(InstallError::from)
}

/// Deletes the files in `folder` that the pack does not name.
fn keep_only(folder: &Path, staged: &pack::Pack) -> Result<(), InstallError> {
    let named = staged.files();
    for entry in fs::read_dir(folder).map_err(io)?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name != MANIFEST && !named.iter().any(|n| *n == name) {
            fs::remove_file(entry.path()).map_err(io)?;
        }
    }
    Ok(())
}

/// Puts `staging` where the pack `id` is, moving an installed one aside first.
fn replace(dir: &Path, id: &str, staging: &Path) -> Result<(), InstallError> {
    let target = dir.join(id);
    let old: Option<PathBuf> = target.exists().then(|| {
        let old = dir.join(format!("{STAGING_PREFIX}remove-{id}"));
        let _ = fs::remove_dir_all(&old);
        old
    });
    if let Some(old) = &old {
        fs::rename(&target, old).map_err(io)?;
    }
    if let Err(e) = fs::rename(staging, &target) {
        // Back as it was.
        if let Some(old) = &old {
            let _ = fs::rename(old, &target);
        }
        return Err(io(e));
    }
    if let Some(old) = old {
        let _ = fs::remove_dir_all(old);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bar::skins::tests::rows_png;
    use crate::bar::skins::zip::tests::zip;

    /// A pack folder `name` in `parent` with a 25 px tile and a stray file.
    fn source_pack(parent: &Path, name: &str) -> PathBuf {
        let folder = parent.join(name);
        fs::create_dir_all(&folder).expect("folder");
        fs::write(
            folder.join(MANIFEST),
            r#"{ "name": "Ocean", "image": "tile.png" }"#,
        )
        .expect("manifest");
        rows_png(&folder.join("tile.png"), 4, &[90; 25]);
        fs::write(folder.join("readme.txt"), "hello").expect("readme");
        folder
    }

    fn files(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(folder)
            .expect("listed")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn folder_is_copied_with_what_it_names() {
        let (from, skins) = (
            tempfile::tempdir().expect("temp"),
            tempfile::tempdir().expect("temp"),
        );
        let source = source_pack(from.path(), "ocean");
        let pack = install(skins.path(), &source).expect("installed");
        assert_eq!((pack.id.as_str(), pack.name.as_str()), ("ocean", "Ocean"));
        assert_eq!(
            files(&skins.path().join("ocean")),
            ["skin.json", "tile.png"]
        );
        // No staging folder is left; the pack is listed.
        assert_eq!(files(skins.path()), ["ocean"]);
        assert!(pack::list(skins.path())[0].pack.is_ok());

        // Installing it again replaces it.
        fs::write(
            source.join(MANIFEST),
            r#"{ "name": "Ocean 2", "image": "tile.png" }"#,
        )
        .expect("manifest");
        let pack = install(skins.path(), &source).expect("installed");
        assert_eq!(pack.name, "Ocean 2");
        assert_eq!(files(skins.path()), ["ocean"]);
    }

    #[test]
    fn bad_packs_are_not_installed() {
        let (from, skins) = (
            tempfile::tempdir().expect("temp"),
            tempfile::tempdir().expect("temp"),
        );
        let empty = from.path().join("empty");
        fs::create_dir_all(&empty).expect("folder");
        assert_eq!(install(skins.path(), &empty), Err(InstallError::NotAPack));
        let source = source_pack(from.path(), "short");
        rows_png(&source.join("tile.png"), 4, &[90; 20]);
        assert!(matches!(
            install(skins.path(), &source),
            Err(InstallError::Pack(PackError::TileHeight { .. }))
        ));
        // A built-in skin's name.
        let gloss = source_pack(from.path(), "gloss");
        assert_eq!(
            install(skins.path(), &gloss),
            Err(InstallError::Pack(PackError::Reserved))
        );
        let hidden = source_pack(from.path(), ".x");
        assert_eq!(
            install(skins.path(), &hidden),
            Err(InstallError::Name(".x".into()))
        );
        // Nothing is left behind.
        assert!(files(skins.path()).is_empty());
        let not_zip = from.path().join("pack.zip");
        fs::write(&not_zip, "hello").expect("written");
        assert_eq!(
            install(skins.path(), &not_zip),
            Err(InstallError::Zip(ZipError::Invalid))
        );
    }

    #[test]
    fn zip_holds_the_pack_at_its_top_or_in_a_folder() {
        let (from, skins) = (
            tempfile::tempdir().expect("temp"),
            tempfile::tempdir().expect("temp"),
        );
        let source = source_pack(from.path(), "src");
        let read = |name: &str| fs::read(source.join(name)).expect("read");
        let (manifest, tile) = (read(MANIFEST), read("tile.png"));

        // In a folder: the folder names the pack; stray and nested files stay out.
        let nested = zip(
            &[
                ("Sunset/", b""),
                ("Sunset/skin.json", &manifest),
                ("Sunset/tile.png", &tile),
                ("Sunset/readme.txt", b"hi"),
                ("Sunset/extra/tile.png", &tile),
                ("other.txt", b"x"),
            ],
            true,
        );
        let path = from.path().join("download.zip");
        fs::write(&path, nested).expect("written");
        let pack = install(skins.path(), &path).expect("installed");
        assert_eq!(pack.id, "Sunset");
        assert_eq!(
            files(&skins.path().join("Sunset")),
            ["skin.json", "tile.png"]
        );

        // At the top: the file names the pack.
        let flat = zip(&[("skin.json", &manifest), ("tile.png", &tile)], false);
        let path = from.path().join("Dusk.zip");
        fs::write(&path, flat).expect("written");
        assert_eq!(
            install(skins.path(), &path).map(|p| p.id),
            Ok("Dusk".into())
        );

        // Deeper, or a tile it names missing: not a pack.
        let deep = zip(&[("a/b/skin.json", &manifest)], false);
        fs::write(&path, deep).expect("written");
        assert_eq!(install(skins.path(), &path), Err(InstallError::NotAPack));
        let no_tile = zip(&[("skin.json", &manifest)], false);
        fs::write(&path, no_tile).expect("written");
        assert_eq!(
            install(skins.path(), &path),
            Err(InstallError::Pack(PackError::Missing("tile.png".into())))
        );
        assert_eq!(files(skins.path()), ["Dusk", "Sunset"]);
    }

    #[test]
    fn only_what_skin_json_names_is_extracted() {
        let (from, skins) = (
            tempfile::tempdir().expect("temp"),
            tempfile::tempdir().expect("temp"),
        );
        let source = source_pack(from.path(), "src");
        let read = |name: &str| fs::read(source.join(name)).expect("read");
        let (manifest, tile) = (read(MANIFEST), read("tile.png"));
        // A stray file compressed with LZMA (method 14), which this reader cannot extract, and
        // many others: never extracted, so they cannot fail the install or fill the disk.
        let mut contents: Vec<(String, Vec<u8>)> = vec![
            ("Sunset/skin.json".into(), manifest.clone()),
            ("Sunset/tile.png".into(), tile),
            ("Sunset/readme.txt".into(), b"read me".to_vec()),
        ];
        contents.extend((0..200).map(|i| (format!("Sunset/f{i}.png"), vec![0; 1000])));
        let entries: Vec<(&str, &[u8])> = contents
            .iter()
            .map(|(n, c)| (n.as_str(), c.as_slice()))
            .collect();
        let mut bytes = zip(&entries, true);
        let name = b"Sunset/readme.txt";
        let found: Vec<usize> = (0..bytes.len() - name.len())
            .filter(|&at| &bytes[at..at + name.len()] == name)
            .collect();
        // The local header's method is 22 bytes before its name, the central one's 36.
        bytes[found[0] - 22] = 14;
        bytes[found[1] - 36] = 14;
        let path = from.path().join("download.zip");
        fs::write(&path, &bytes).expect("written");
        let pack = install(skins.path(), &path).expect("installed");
        assert_eq!(pack.id, "Sunset");
        assert_eq!(
            files(&skins.path().join("Sunset")),
            ["skin.json", "tile.png"]
        );

        // A named tile over 1 MB is refused before it is extracted.
        let big = vec![0; 1024 * 1024 + 1];
        let bytes = zip(&[("skin.json", &manifest), ("tile.png", &big)], false);
        let path = from.path().join("Big.zip");
        fs::write(&path, bytes).expect("written");
        assert_eq!(
            install(skins.path(), &path),
            Err(InstallError::Pack(PackError::TooLarge("tile.png".into())))
        );
        assert_eq!(files(skins.path()), ["Sunset"]);
    }

    /// A `.zip` another tool wrote (Python's `zipfile`, deflated, with a folder entry).
    #[test]
    fn zip_written_elsewhere() {
        let skins = tempfile::tempdir().expect("temp");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skins/sunset.zip");
        let pack = install(skins.path(), &fixture).expect("installed");
        assert_eq!((pack.id.as_str(), pack.shadow_band), ("Sunset", 4));
        assert_eq!(
            files(&skins.path().join("Sunset")),
            ["skin.json", "tile.png"]
        );
    }
}

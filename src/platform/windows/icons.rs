//! Shell icons (SPEC §4.6 `Icons`): the icon Explorer shows for a file, folder or drive, at a
//! physical size, from `IShellItemImageFactory` (a `.lnk` gives its target's icon). It is
//! called on the bar's icon worker, so a slow network path never holds up the bar.

use std::mem::size_of;

use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, GetDIBits, GetObjectW, HBITMAP, HGDIOBJ,
};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_ICONONLY,
};
use windows::core::HSTRING;

use super::disk::QuietErrors;
use crate::platform::Bitmap;

/// Larger icons are not asked for (the shell's largest is 256 px).
const MAX_PX: u32 = 256;

pub fn shell_icon(path: &str, px: u32) -> Option<Bitmap> {
    if px == 0 || px > MAX_PX || path.is_empty() {
        return None;
    }
    super::com::enter();
    // An empty card reader or optical drive must not ask for a disk.
    let _quiet = QuietErrors::new();
    let name = HSTRING::from(path);
    // SAFETY: a valid path string; no bind context.
    let factory: IShellItemImageFactory = match unsafe { SHCreateItemFromParsingName(&name, None) }
    {
        Ok(factory) => factory,
        Err(e) => {
            tracing::debug!("no shell item for {path:?}: {e}");
            return None;
        }
    };
    let size = SIZE {
        cx: px as i32,
        cy: px as i32,
    };
    // SAFETY: plain COM call; the bitmap is ours and deleted below.
    let hbitmap = match unsafe { factory.GetImage(size, SIIGBF_ICONONLY) } {
        Ok(hbitmap) => hbitmap,
        Err(e) => {
            tracing::debug!("no shell icon for {path:?}: {e}");
            return None;
        }
    };
    let bitmap = read(hbitmap);
    // SAFETY: the bitmap GetImage handed over, no longer used.
    let _ = unsafe { DeleteObject(HGDIOBJ(hbitmap.0)) };
    bitmap
}

/// The pixels of a 32-bit `hbitmap`, top-down.
fn read(hbitmap: HBITMAP) -> Option<Bitmap> {
    let mut info = BITMAP::default();
    // SAFETY: `info` is a writable BITMAP and its size is given.
    let got = unsafe {
        GetObjectW(
            HGDIOBJ(hbitmap.0),
            size_of::<BITMAP>() as i32,
            Some((&raw mut info).cast()),
        )
    };
    let (width, height) = (info.bmWidth, info.bmHeight.abs());
    if got == 0 || width <= 0 || height <= 0 || width as u32 > MAX_PX || height as u32 > MAX_PX {
        return None;
    }
    let mut header = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            // Negative: top-down rows.
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bgra = vec![0u8; width as usize * height as usize * 4];
    // SAFETY: a memory DC for GetDIBits, deleted below; the buffer holds `height` rows of
    // `width` 32-bit pixels, as the header asks for.
    let lines = unsafe {
        let dc = CreateCompatibleDC(None);
        let lines = GetDIBits(
            dc,
            hbitmap,
            0,
            height as u32,
            Some(bgra.as_mut_ptr().cast()),
            &mut header,
            DIB_RGB_COLORS,
        );
        let _ = DeleteDC(dc);
        lines
    };
    if lines != height {
        return None;
    }
    Bitmap::from_shell_bgra(width as u32, height as u32, bgra)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs on the Windows CI runner (`cargo test -- --ignored`).
    #[test]
    #[ignore = "reads this machine's shell icons"]
    fn a_program_and_a_folder_have_icons() {
        let windir = std::env::var("windir").unwrap_or_else(|_| "C:\\Windows".into());
        for path in [
            format!("{windir}\\notepad.exe"),
            windir.clone(),
            "C:\\".into(),
        ] {
            let icon = shell_icon(&path, 24).unwrap_or_else(|| panic!("an icon for {path}"));
            assert_eq!((icon.width(), icon.height()), (24, 24), "{path}");
            assert!(icon.pixels().as_chunks::<4>().0.iter().any(|px| px[3] > 0));
        }
        assert!(shell_icon("Q:\\no\\such\\file.exe", 24).is_none());
    }
}

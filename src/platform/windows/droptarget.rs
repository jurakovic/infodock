//! The bar's drop target (SPEC §6.3): an `IDropTarget` registered on the bar window
//! (`RegisterDragDrop`, bar thread). It reads paths (`CF_HDROP`, or the `Shell IDList Array`
//! for items with a file-system path) and web addresses (`UniformResourceLocatorW`, with the
//! link's text from `FileGroupDescriptorW` or the `HTML Format` fragment), and hands the bar
//! logic enter, over, leave and drop events, which answer with the effect. The shell's drag
//! image keeps showing over the bar through `IDropTargetHelper`.

use std::cell::OnceCell;

use windows::Win32::Foundation::{GlobalFree, HWND, POINT, POINTL};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, DVASPECT_CONTENT, FORMATETC, IDataObject, STGMEDIUM,
    TYMED_HGLOBAL,
};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows::Win32::System::Ole::{
    CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DROPEFFECT_NONE,
    IDropTarget, IDropTarget_Impl, MK_ALT, RegisterDragDrop, ReleaseStgMedium,
};
use windows::Win32::System::SystemServices::{MK_CONTROL, MK_SHIFT, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    CLSID_DragDropHelper, IDropTargetHelper, ILCombine, ILFree, SHGetPathFromIDListW,
};
use windows::core::{PCWSTR, Ref, implement, w};

use crate::platform::drag::{self, DragEvent, DragItem, DropEffect, DropEffects};
use crate::platform::{Modifiers, PointF};

/// Where the bar logic gets the drag events, on the bar thread.
pub(super) type DragSink = Box<dyn Fn(DragEvent) -> DropEffect>;

/// Registers the bar window as a drop target; failures are logged (drops are then refused).
/// The thread must have called `OleInitialize`.
pub(super) fn register(hwnd: HWND, sink: DragSink) {
    let target: IDropTarget = BarDropTarget {
        hwnd,
        sink,
        helper: OnceCell::new(),
    }
    .into();
    // SAFETY: our own window on this thread; OLE keeps a reference to the target.
    if let Err(e) = unsafe { RegisterDragDrop(hwnd, &target) } {
        tracing::warn!("the bar cannot take drops: {e}");
    }
}

#[implement(IDropTarget)]
struct BarDropTarget {
    hwnd: HWND,
    sink: DragSink,
    /// The shell's drag image helper, created at the first drag (`None` if it cannot be).
    helper: OnceCell<Option<IDropTargetHelper>>,
}

impl BarDropTarget {
    fn helper(&self) -> Option<&IDropTargetHelper> {
        self.helper
            .get_or_init(|| {
                // SAFETY: creates the shell's in-process helper; failing only loses the image.
                unsafe { CoCreateInstance(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER) }
                    .map_err(|e| tracing::info!("no drag image over the bar: {e}"))
                    .ok()
            })
            .as_ref()
    }

    /// `pt` (physical screen px) in logical px from the bar window's top-left corner.
    fn point(&self, pt: &POINTL) -> PointF {
        let mut point = POINT { x: pt.x, y: pt.y };
        // SAFETY: plain queries on our own window.
        let scale = unsafe {
            let _ = ScreenToClient(self.hwnd, &mut point);
            GetDpiForWindow(self.hwnd).max(96) as f32 / 96.0
        };
        PointF::new(point.x as f32 / scale, point.y as f32 / scale)
    }

    /// Sends `event` to the bar logic and writes the effect it answers, limited to what the
    /// source allows, into `effect`.
    fn answer(&self, event: DragEvent, allowed: DROPEFFECT, effect: *mut DROPEFFECT) -> DROPEFFECT {
        let answer = to_dropeffect((self.sink)(event));
        let answer = if allowed.0 & answer.0 == answer.0 {
            answer
        } else {
            DROPEFFECT_NONE
        };
        // SAFETY: OLE passes a valid in/out pointer (checked for null all the same).
        if let Some(effect) = unsafe { effect.as_mut() } {
            *effect = answer;
        }
        answer
    }
}

impl IDropTarget_Impl for BarDropTarget_Impl {
    fn DragEnter(
        &self,
        data: Ref<IDataObject>,
        keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        // SAFETY: OLE passes a valid in/out pointer (checked for null all the same).
        let allowed = unsafe { effect.as_ref() }
            .copied()
            .unwrap_or(DROPEFFECT_NONE);
        let items = data.ok().map(read_items).unwrap_or_default();
        let event = DragEvent::Enter {
            items,
            at: self.point(pt),
            modifiers: modifiers(keys),
            allowed: from_dropeffect(allowed),
        };
        let answer = self.answer(event, allowed, effect);
        if let (Some(helper), Ok(data)) = (self.helper(), data.ok()) {
            let point = POINT { x: pt.x, y: pt.y };
            // SAFETY: the helper draws the drag image; arguments are valid for the call.
            let _ = unsafe { helper.DragEnter(self.hwnd, data, &point, answer) };
        }
        Ok(())
    }

    fn DragOver(
        &self,
        keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        // SAFETY: as in DragEnter.
        let allowed = unsafe { effect.as_ref() }
            .copied()
            .unwrap_or(DROPEFFECT_NONE);
        let event = DragEvent::Over {
            at: self.point(pt),
            modifiers: modifiers(keys),
            allowed: from_dropeffect(allowed),
        };
        let answer = self.answer(event, allowed, effect);
        if let Some(helper) = self.helper.get().and_then(Option::as_ref) {
            let point = POINT { x: pt.x, y: pt.y };
            // SAFETY: as in DragEnter.
            let _ = unsafe { helper.DragOver(&point, answer) };
        }
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        (self.sink)(DragEvent::Leave);
        if let Some(helper) = self.helper.get().and_then(Option::as_ref) {
            // SAFETY: ends what DragEnter started.
            let _ = unsafe { helper.DragLeave() };
        }
        Ok(())
    }

    fn Drop(
        &self,
        data: Ref<IDataObject>,
        keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        // SAFETY: as in DragEnter.
        let allowed = unsafe { effect.as_ref() }
            .copied()
            .unwrap_or(DROPEFFECT_NONE);
        // The helper first: the drop may open a dialog, which should not show the drag image.
        if let (Some(helper), Ok(data)) = (self.helper(), data.ok()) {
            let point = POINT { x: pt.x, y: pt.y };
            // SAFETY: as in DragEnter.
            let _ = unsafe { helper.Drop(data, &point, allowed) };
        }
        let event = DragEvent::Drop {
            at: self.point(pt),
            modifiers: modifiers(keys),
            allowed: from_dropeffect(allowed),
        };
        if self.answer(event, allowed, effect) == DROPEFFECT_MOVE {
            // The bar moves the items itself (a Folder Menu, the Recycle Bin): an optimized
            // move, so the source must not delete them as well.
            if let Ok(data) = data.ok() {
                report_optimized_move(data);
            }
            // SAFETY: as in DragEnter.
            if let Some(effect) = unsafe { effect.as_mut() } {
                *effect = DROPEFFECT_NONE;
            }
        }
        Ok(())
    }
}

fn modifiers(keys: MODIFIERKEYS_FLAGS) -> Modifiers {
    Modifiers {
        alt: keys.0 & MK_ALT != 0,
        ctrl: keys.0 & MK_CONTROL.0 != 0,
        shift: keys.0 & MK_SHIFT.0 != 0,
    }
}

fn from_dropeffect(effect: DROPEFFECT) -> DropEffects {
    DropEffects {
        copy: effect.0 & DROPEFFECT_COPY.0 != 0,
        moves: effect.0 & DROPEFFECT_MOVE.0 != 0,
        link: effect.0 & DROPEFFECT_LINK.0 != 0,
    }
}

fn to_dropeffect(effect: DropEffect) -> DROPEFFECT {
    match effect {
        DropEffect::None => DROPEFFECT_NONE,
        DropEffect::Copy => DROPEFFECT_COPY,
        DropEffect::Move => DROPEFFECT_MOVE,
        DropEffect::Link => DROPEFFECT_LINK,
    }
}

/// Tells the source that the target moved the items itself (the shell's "optimized move"):
/// `Performed DropEffect` none (the source deletes nothing), `Logical Performed DropEffect`
/// move (what happened, for its display).
fn report_optimized_move(data: &IDataObject) {
    set_effect(data, format(w!("Performed DropEffect")), DROPEFFECT_NONE);
    set_effect(
        data,
        format(w!("Logical Performed DropEffect")),
        DROPEFFECT_MOVE,
    );
}

/// Hands `data` a `DROPEFFECT` in clipboard format `format`.
fn set_effect(data: &IDataObject, format: u16, effect: DROPEFFECT) {
    if format == 0 {
        return;
    }
    // SAFETY: the global holds one u32 written while locked; the data object owns it after a
    // successful SetData, and it is freed here otherwise.
    unsafe {
        let Ok(global) = GlobalAlloc(GMEM_MOVEABLE, size_of::<u32>()) else {
            return;
        };
        let pointer = GlobalLock(global);
        if pointer.is_null() {
            let _ = GlobalFree(Some(global));
            return;
        }
        pointer.cast::<u32>().write_unaligned(effect.0);
        let _ = GlobalUnlock(global);
        let request = FORMATETC {
            cfFormat: format,
            ptd: std::ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT.0,
            lindex: -1,
            tymed: TYMED_HGLOBAL.0 as u32,
        };
        let mut medium = STGMEDIUM {
            tymed: TYMED_HGLOBAL.0 as u32,
            ..Default::default()
        };
        medium.u.hGlobal = global;
        if let Err(e) = data.SetData(&request, &medium, true) {
            tracing::debug!("the drag source takes no drop result: {e}");
            let _ = GlobalFree(Some(global));
        }
    }
}

/// A registered clipboard format's id; 0 if it cannot be registered.
fn format(name: PCWSTR) -> u16 {
    // SAFETY: a static NUL-terminated name.
    u16::try_from(unsafe { RegisterClipboardFormatW(name) }).unwrap_or(0)
}

/// The bytes `data` offers in clipboard format `format` as global memory.
fn global_bytes(data: &IDataObject, format: u16) -> Option<Vec<u8>> {
    if format == 0 {
        return None;
    }
    let request = FORMATETC {
        cfFormat: format,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    // SAFETY: the medium is released below; the global memory is read only while locked.
    unsafe {
        let mut medium = data.GetData(&request).ok()?;
        let mut bytes = None;
        if medium.tymed == TYMED_HGLOBAL.0 as u32 {
            let global = medium.u.hGlobal;
            let pointer = GlobalLock(global);
            if !pointer.is_null() {
                let size = GlobalSize(global);
                bytes = Some(std::slice::from_raw_parts(pointer.cast::<u8>(), size).to_vec());
                let _ = GlobalUnlock(global);
            }
        }
        ReleaseStgMedium(&mut medium);
        bytes
    }
}

/// What `data` holds that the bar may take (SPEC §6.3): paths, or else a web address.
fn read_items(data: &IDataObject) -> Vec<DragItem> {
    let paths = global_bytes(data, CF_HDROP.0)
        .map(|bytes| drag::dropfiles(&bytes))
        .filter(|paths| !paths.is_empty())
        .or_else(|| shell_id_paths(data));
    if let Some(paths) = paths.filter(|p| !p.is_empty()) {
        return paths.into_iter().map(DragItem::Path).collect();
    }
    let url = global_bytes(data, format(w!("UniformResourceLocatorW")))
        .map(|b| drag::wide_string(&b))
        .or_else(|| {
            global_bytes(data, format(w!("UniformResourceLocator")))
                .map(|b| drag::narrow_string(&b))
        })
        .filter(|url| !url.trim().is_empty());
    let Some(url) = url else {
        return Vec::new();
    };
    let title = global_bytes(data, format(w!("FileGroupDescriptorW")))
        .and_then(|b| drag::link_file_title(&b))
        .or_else(|| {
            global_bytes(data, format(w!("HTML Format")))
                .and_then(|b| drag::html_link_text(&String::from_utf8_lossy(&b)))
        });
    vec![DragItem::Url { url, title }]
}

/// The file-system paths of a `Shell IDList Array` (`CIDA`: the count, then the offsets of
/// the parent folder's and each item's ID list), for items without a `CF_HDROP`.
fn shell_id_paths(data: &IDataObject) -> Option<Vec<String>> {
    let bytes = global_bytes(data, format(w!("Shell IDList Array")))?;
    let word = |i: usize| {
        bytes
            .get(i * 4..i * 4 + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    };
    let count = word(0)?;
    let parent = word(1)?;
    let pidl = |offset: usize| -> Option<*const ITEMIDLIST> {
        drag::id_list_fits(&bytes, offset).then(|| bytes[offset..].as_ptr().cast::<ITEMIDLIST>())
    };
    let parent = pidl(parent)?;
    let mut paths = Vec::new();
    for i in 0..count.min(1024) {
        let Some(child) = word(2 + i).and_then(pidl) else {
            break;
        };
        // SAFETY: both ID lists end within `bytes` (checked above); the combined list is
        // freed after use.
        unsafe {
            let full = ILCombine(Some(parent), Some(child));
            if full.is_null() {
                continue;
            }
            let mut buffer = [0u16; 260];
            if SHGetPathFromIDListW(full, &mut buffer).as_bool() {
                paths.push(super::from_wide(&buffer));
            }
            ILFree(Some(full));
        }
    }
    Some(paths)
}

#[cfg(test)]
mod tests {
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::Memory::GHND;
    use windows::Win32::UI::Shell::SHCreateDataObject;

    use super::*;

    /// A shell data object offering `formats` as global memory.
    fn data_object(formats: &[(u16, Vec<u8>)]) -> IDataObject {
        // SAFETY: an empty shell data object; each SetData hands it a global it then owns.
        unsafe {
            let data: IDataObject = SHCreateDataObject(None, None, None).expect("a data object");
            for (format, bytes) in formats {
                let global: HGLOBAL = GlobalAlloc(GHND, bytes.len()).expect("memory");
                let pointer = GlobalLock(global);
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast::<u8>(), bytes.len());
                let _ = GlobalUnlock(global);
                let request = FORMATETC {
                    cfFormat: *format,
                    ptd: std::ptr::null_mut(),
                    dwAspect: DVASPECT_CONTENT.0,
                    lindex: -1,
                    tymed: TYMED_HGLOBAL.0 as u32,
                };
                let mut medium = STGMEDIUM {
                    tymed: TYMED_HGLOBAL.0 as u32,
                    ..Default::default()
                };
                medium.u.hGlobal = global;
                data.SetData(&request, &medium, true).expect("set");
            }
            data
        }
    }

    fn wide(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    /// Runs on the Windows CI runner (`cargo test -- --ignored`): reads what Explorer and
    /// browsers put in a drag through a real shell data object.
    #[test]
    #[ignore = "uses the Windows shell"]
    fn reads_paths_and_links_from_a_data_object() {
        super::super::com::enter();
        let mut hdrop = vec![20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0];
        hdrop.extend(wide("C:\\Windows\0E:\\\0\0"));
        let files = data_object(&[(CF_HDROP.0, hdrop)]);
        assert_eq!(
            read_items(&files),
            [
                DragItem::Path("C:\\Windows".into()),
                DragItem::Path("E:\\".into())
            ]
        );
        let mut group = vec![1, 0, 0, 0];
        group.extend(vec![0; 72]);
        let mut name = wide("Example Domain.url");
        name.resize(520, 0);
        group.extend(name);
        let link = data_object(&[
            (
                format(w!("UniformResourceLocatorW")),
                wide("https://example.com/\0"),
            ),
            (format(w!("FileGroupDescriptorW")), group),
        ]);
        assert_eq!(
            read_items(&link),
            [DragItem::Url {
                url: "https://example.com/".into(),
                title: Some("Example Domain".into())
            }]
        );
        let html = data_object(&[
            (
                format(w!("UniformResourceLocator")),
                b"https://example.org/\0".to_vec(),
            ),
            (
                format(w!("HTML Format")),
                b"<a href=\"https://example.org/\">Example</a>".to_vec(),
            ),
        ]);
        assert_eq!(
            read_items(&html),
            [DragItem::Url {
                url: "https://example.org/".into(),
                title: Some("Example".into())
            }]
        );
        assert!(read_items(&data_object(&[])).is_empty());
        // After a move the bar did itself, the source reads that it deletes nothing.
        let moved = data_object(&[]);
        report_optimized_move(&moved);
        let performed = global_bytes(&moved, format(w!("Performed DropEffect")));
        assert_eq!(
            performed.as_deref().map(|b| &b[..4]),
            Some(&[0u8, 0, 0, 0][..])
        );
        let logical = global_bytes(&moved, format(w!("Logical Performed DropEffect")));
        assert_eq!(
            logical.as_deref().map(|b| &b[..4]),
            Some(&[2u8, 0, 0, 0][..])
        );
    }
}

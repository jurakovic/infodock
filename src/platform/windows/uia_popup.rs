//! The popup window as a UI Automation provider (SPEC §14.4): a list whose items are the open
//! popup's rows (the Folder Menu's items, the Eject USB list's drives). As for the bar
//! (`uia.rs`), the providers answer from the description the bar logic hands over with each of
//! the popup's frames ([`AccessiblePopup`]), and comparing each description with the last
//! raises the events screen readers follow: the mark moving (selection and focus), rows coming
//! and going (structure), another folder (name). A row a screen reader selects is queued as
//! [`AccessibleAction::SelectRow`] and the window is told with [`WM_UIA`].

// The Win32 constants keep their names, also as match patterns.
#![allow(non_upper_case_globals)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use windows::Win32::Foundation::{E_OUTOFMEMORY, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::Com::SAFEARRAY;
use windows::Win32::System::Ole::{SafeArrayCreateVector, SafeArrayDestroy, SafeArrayPutElement};
use windows::Win32::System::Variant::{VARIANT, VT_UNKNOWN};
use windows::Win32::UI::Accessibility::{
    IRawElementProviderFragment, IRawElementProviderFragment_Impl, IRawElementProviderFragmentRoot,
    IRawElementProviderFragmentRoot_Impl, IRawElementProviderSimple,
    IRawElementProviderSimple_Impl, ISelectionItemProvider, ISelectionItemProvider_Impl,
    ISelectionProvider, ISelectionProvider_Impl, NavigateDirection, NavigateDirection_FirstChild,
    NavigateDirection_LastChild, NavigateDirection_NextSibling, NavigateDirection_Parent,
    NavigateDirection_PreviousSibling, ProviderOptions, ProviderOptions_ServerSideProvider,
    ProviderOptions_UseComThreading, StructureChangeType_ChildrenInvalidated,
    UIA_AutomationFocusChangedEventId, UIA_AutomationIdPropertyId, UIA_ControlTypePropertyId,
    UIA_E_INVALIDOPERATION, UIA_HasKeyboardFocusPropertyId, UIA_IsEnabledPropertyId,
    UIA_IsKeyboardFocusablePropertyId, UIA_IsOffscreenPropertyId, UIA_ListControlTypeId,
    UIA_ListItemControlTypeId, UIA_NamePropertyId, UIA_PATTERN_ID, UIA_PROPERTY_ID,
    UIA_PositionInSetPropertyId, UIA_SelectionItem_ElementSelectedEventId,
    UIA_SelectionItemPatternId, UIA_SelectionPatternId, UIA_SizeOfSetPropertyId,
    UiaClientsAreListening, UiaHostProviderFromHwnd, UiaRaiseAutomationEvent,
    UiaRaiseAutomationPropertyChangedEvent, UiaRaiseStructureChangedEvent, UiaRect,
    UiaReturnRawElementProvider,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowRect, PostMessageW};
use windows::core::{BOOL, Error, HRESULT, IUnknown, IUnknownImpl, Interface, Result, implement};

use super::uia::{HWND_RUNTIME_ID, Owned, WM_UIA, boolean, gone, int, none, runtime_array, text};
use crate::platform::accessible::{self, PopupEvent};
use crate::platform::{AccessibleAction, AccessiblePopup, AccessibleRow, RectF};

/// The popup list's automation id; its rows have none (they come and go with folders).
const POPUP_ID: &str = "InfodockPopup";

type Shared = Rc<RefCell<Tree>>;

struct Tree {
    hwnd: HWND,
    /// The shown popup; `None` while the window is hidden.
    popup: Option<AccessiblePopup>,
    /// Physical px per logical px of the shown content.
    scale: f64,
    /// The shown rows' runtime ids, given when a row is first seen and kept while it shows.
    runtime: HashMap<String, i32>,
    /// The next runtime id: never given twice, so a row that went keeps its id to itself.
    next: i32,
    actions: Vec<AccessibleAction>,
}

impl Tree {
    fn popup(&self) -> Result<&AccessiblePopup> {
        self.popup.as_ref().ok_or_else(gone)
    }

    fn row(&self, id: &str) -> Result<(usize, &AccessibleRow)> {
        self.popup()?.row(id).ok_or_else(gone)
    }

    /// Queues `action` for the bar logic and tells the window.
    fn ask(&mut self, action: AccessibleAction) {
        if self.actions.is_empty() {
            // SAFETY: posting to the popup window, which lives as long as the process.
            let _ = unsafe { PostMessageW(Some(self.hwnd), WM_UIA, WPARAM(0), LPARAM(0)) };
        }
        self.actions.push(action);
    }

    /// Gives the shown rows runtime ids and forgets those of rows that went.
    fn number(&mut self) {
        let Some(popup) = &self.popup else {
            self.runtime.clear();
            return;
        };
        self.runtime
            .retain(|id, _| popup.rows.iter().any(|r| r.id == *id));
        for row in &popup.rows {
            if !self.runtime.contains_key(&row.id) {
                self.next = self.next.checked_add(1).unwrap_or(1);
                self.runtime.insert(row.id.clone(), self.next);
            }
        }
    }

    /// The window's top-left corner, physical screen px.
    fn origin(&self) -> (f64, f64) {
        let mut window = RECT::default();
        // SAFETY: a plain query on the popup window.
        let _ = unsafe { GetWindowRect(self.hwnd, &mut window) };
        (f64::from(window.left), f64::from(window.top))
    }

    /// `rect` (logical px in the window) in physical screen px.
    fn screen(&self, rect: RectF) -> UiaRect {
        let (x, y) = self.origin();
        UiaRect {
            left: x + f64::from(rect.left) * self.scale,
            top: y + f64::from(rect.top) * self.scale,
            width: f64::from(rect.width()) * self.scale,
            height: f64::from(rect.height()) * self.scale,
        }
    }

    /// Whether the popup window has the keyboard (it takes it when it appears).
    fn focused(&self) -> bool {
        // SAFETY: a plain query.
        self.popup.is_some() && unsafe { GetForegroundWindow() } == self.hwnd
    }
}

/// The popup window's UI Automation provider, created at the first `WM_GETOBJECT` that asks for
/// it; the window is reused, and so is the provider.
pub(super) struct PopupUia {
    root: IRawElementProviderSimple,
    tree: Shared,
}

impl PopupUia {
    pub(super) fn new(hwnd: HWND) -> Self {
        let tree = Rc::new(RefCell::new(Tree {
            hwnd,
            popup: None,
            scale: 1.0,
            runtime: HashMap::new(),
            next: 0,
            actions: Vec::new(),
        }));
        let root = list(&tree).into();
        Self { root, tree }
    }

    /// Answers a `WM_GETOBJECT` for `UiaRootObjectId` with the list provider.
    pub(super) fn answer(&self, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let hwnd = self.tree.borrow().hwnd;
        // SAFETY: our own window and provider, on the window's thread.
        unsafe { UiaReturnRawElementProvider(hwnd, wparam, lparam, &self.root) }
    }

    /// Takes the description of the popup as just drawn at `scale` (`appeared`: it was hidden)
    /// and raises the events that tell listening screen readers what changed.
    pub(super) fn update(&self, popup: AccessiblePopup, scale: f32, appeared: bool) {
        let events = {
            let mut tree = self.tree.borrow_mut();
            let old = if appeared { None } else { tree.popup.take() };
            let events = accessible::popup_events(old.as_ref(), &popup);
            tree.popup = Some(popup);
            tree.scale = f64::from(scale.max(0.1));
            tree.number();
            events
        };
        // SAFETY: plain query.
        if events.is_empty() || !unsafe { UiaClientsAreListening() }.as_bool() {
            return;
        }
        // Raised with the description released: in-process clients may ask at once.
        for event in events {
            if let Err(e) = self.raise(event) {
                tracing::debug!("a UI Automation event could not be raised: {e}");
            }
        }
    }

    /// The window was hidden: its rows are gone.
    pub(super) fn clear(&self) {
        let mut tree = self.tree.borrow_mut();
        tree.popup = None;
        tree.number();
    }

    fn raise(&self, event: PopupEvent) -> Result<()> {
        let focused = self.tree.borrow().focused();
        // SAFETY: our own providers; the variants are freed after the calls.
        unsafe {
            match event {
                PopupEvent::Structure => {
                    let hwnd = self.tree.borrow().hwnd;
                    let mut id = [HWND_RUNTIME_ID, hwnd.0 as isize as i32];
                    UiaRaiseStructureChangedEvent(
                        &self.root,
                        StructureChangeType_ChildrenInvalidated,
                        id.as_mut_ptr(),
                        id.len() as i32,
                    )
                }
                PopupEvent::Renamed { old, new } => {
                    let (old, new) = (Owned(text(&old)), Owned(text(&new)));
                    UiaRaiseAutomationPropertyChangedEvent(
                        &self.root,
                        UIA_NamePropertyId,
                        &old.0,
                        &new.0,
                    )?;
                    // Into another folder with nothing marked: the list is read again.
                    let unmarked = self
                        .tree
                        .borrow()
                        .popup
                        .as_ref()
                        .is_some_and(|p| p.selected.is_none());
                    if focused && unmarked {
                        UiaRaiseAutomationEvent(&self.root, UIA_AutomationFocusChangedEventId)?;
                    }
                    Ok(())
                }
                PopupEvent::Selected(id) => {
                    let row: IRawElementProviderSimple = row(&self.tree, &id).into();
                    UiaRaiseAutomationEvent(&row, UIA_SelectionItem_ElementSelectedEventId)?;
                    if focused {
                        UiaRaiseAutomationEvent(&row, UIA_AutomationFocusChangedEventId)?;
                    }
                    Ok(())
                }
            }
        }
    }

    /// What screen readers asked since the last call.
    pub(super) fn take_actions(&self) -> Vec<AccessibleAction> {
        std::mem::take(&mut self.tree.borrow_mut().actions)
    }
}

fn list(tree: &Shared) -> ListProvider {
    ListProvider {
        tree: Rc::clone(tree),
    }
}

fn row(tree: &Shared, id: &str) -> RowProvider {
    RowProvider {
        tree: Rc::clone(tree),
        id: id.to_owned(),
    }
}

/// The popup window: a list of the popup's rows, named as the window, whose selection is the
/// marked row. The window's own provider adds its place, focus and parent.
#[implement(
    IRawElementProviderSimple,
    IRawElementProviderFragment,
    IRawElementProviderFragmentRoot,
    ISelectionProvider
)]
struct ListProvider {
    tree: Shared,
}

impl IRawElementProviderSimple_Impl for ListProvider_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading)
    }

    fn GetPatternProvider(&self, pattern: UIA_PATTERN_ID) -> Result<IUnknown> {
        match pattern {
            UIA_SelectionPatternId => Ok(self.to_interface::<ISelectionProvider>().into()),
            _ => none(),
        }
    }

    fn GetPropertyValue(&self, property: UIA_PROPERTY_ID) -> Result<VARIANT> {
        let tree = self.tree.borrow();
        Ok(match property {
            UIA_ControlTypePropertyId => int(UIA_ListControlTypeId.0),
            UIA_AutomationIdPropertyId => text(POPUP_ID),
            // Otherwise the window's title, which is the same.
            UIA_NamePropertyId => match &tree.popup {
                Some(popup) if !popup.name.is_empty() => text(&popup.name),
                _ => VARIANT::default(),
            },
            _ => VARIANT::default(),
        })
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        let hwnd = self.tree.borrow().hwnd;
        // SAFETY: our own window.
        unsafe { UiaHostProviderFromHwnd(hwnd) }
    }
}

impl IRawElementProviderFragment_Impl for ListProvider_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        let tree = self.tree.borrow();
        let rows = tree
            .popup
            .as_ref()
            .map(|p| p.rows.as_slice())
            .unwrap_or_default();
        let child = match direction {
            NavigateDirection_FirstChild => rows.first(),
            NavigateDirection_LastChild => rows.last(),
            // The window's parent and siblings come from its host provider.
            _ => None,
        };
        match child {
            Some(r) => Ok(row(&self.tree, &r.id).into()),
            None => none(),
        }
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        // The window's own.
        Ok(std::ptr::null_mut())
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        // The window's own.
        Ok(UiaRect::default())
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        Ok(())
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        Ok(self.to_interface())
    }
}

impl IRawElementProviderFragmentRoot_Impl for ListProvider_Impl {
    fn ElementProviderFromPoint(&self, x: f64, y: f64) -> Result<IRawElementProviderFragment> {
        let tree = self.tree.borrow();
        let (left, top) = tree.origin();
        let (x, y) = (
            ((x - left) / tree.scale) as f32,
            ((y - top) / tree.scale) as f32,
        );
        match tree.popup.as_ref().and_then(|p| p.row_at(x, y)) {
            Some(r) => Ok(row(&self.tree, &r.id).into()),
            None => Ok(self.to_interface()),
        }
    }

    fn GetFocus(&self) -> Result<IRawElementProviderFragment> {
        let tree = self.tree.borrow();
        match tree.popup.as_ref().and_then(AccessiblePopup::selected_row) {
            Some(r) => Ok(row(&self.tree, &r.id).into()),
            None => none(),
        }
    }
}

impl ISelectionProvider_Impl for ListProvider_Impl {
    fn GetSelection(&self) -> Result<*mut SAFEARRAY> {
        let selected = {
            let tree = self.tree.borrow();
            tree.popup
                .as_ref()
                .and_then(AccessiblePopup::selected_row)
                .map(|r| r.id.clone())
        };
        let rows: Vec<IRawElementProviderSimple> = selected
            .iter()
            .map(|id| row(&self.tree, id).into())
            .collect();
        // SAFETY: a new vector of interface pointers, each added (and AddRef'd) by index;
        // destroyed again on failure.
        unsafe {
            let array = SafeArrayCreateVector(VT_UNKNOWN, 0, rows.len() as u32);
            if array.is_null() {
                return Err(E_OUTOFMEMORY.into());
            }
            for (index, provider) in rows.iter().enumerate() {
                let index = index as i32;
                if let Err(e) = SafeArrayPutElement(array, &index, provider.as_raw()) {
                    let _ = SafeArrayDestroy(array);
                    return Err(e);
                }
            }
            Ok(array)
        }
    }

    fn CanSelectMultiple(&self) -> Result<BOOL> {
        Ok(false.into())
    }

    fn IsSelectionRequired(&self) -> Result<BOOL> {
        Ok(false.into())
    }
}

/// A row of the popup: a list item, named as the popup reads it, its place in the list and on
/// the screen, selected while marked.
#[implement(
    IRawElementProviderSimple,
    IRawElementProviderFragment,
    ISelectionItemProvider
)]
struct RowProvider {
    tree: Shared,
    id: String,
}

impl RowProvider_Impl {
    /// Asks the bar logic to mark this row.
    fn select(&self) -> Result<()> {
        let mut tree = self.tree.borrow_mut();
        tree.row(&self.id)?;
        tree.ask(AccessibleAction::SelectRow(self.id.clone()));
        Ok(())
    }

    fn selected(&self) -> Result<bool> {
        let tree = self.tree.borrow();
        let (index, _) = tree.row(&self.id)?;
        Ok(tree.popup()?.selected == Some(index))
    }
}

fn invalid_operation() -> Error {
    Error::from(HRESULT(UIA_E_INVALIDOPERATION as i32))
}

impl IRawElementProviderSimple_Impl for RowProvider_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading)
    }

    fn GetPatternProvider(&self, pattern: UIA_PATTERN_ID) -> Result<IUnknown> {
        self.tree.borrow().row(&self.id)?;
        match pattern {
            UIA_SelectionItemPatternId => Ok(self.to_interface::<ISelectionItemProvider>().into()),
            _ => none(),
        }
    }

    fn GetPropertyValue(&self, property: UIA_PROPERTY_ID) -> Result<VARIANT> {
        let tree = self.tree.borrow();
        let popup = tree.popup()?;
        let (index, row) = tree.row(&self.id)?;
        let count = i32::try_from(popup.rows.len()).unwrap_or(i32::MAX);
        Ok(match property {
            UIA_ControlTypePropertyId => int(UIA_ListItemControlTypeId.0),
            UIA_NamePropertyId => text(&row.name),
            UIA_PositionInSetPropertyId => int(i32::try_from(index + 1).unwrap_or(i32::MAX)),
            UIA_SizeOfSetPropertyId => int(count),
            UIA_IsOffscreenPropertyId => boolean(row.rect.is_none()),
            UIA_IsKeyboardFocusablePropertyId | UIA_IsEnabledPropertyId => boolean(true),
            UIA_HasKeyboardFocusPropertyId => {
                boolean(tree.focused() && popup.selected == Some(index))
            }
            _ => VARIANT::default(),
        })
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        none()
    }
}

impl IRawElementProviderFragment_Impl for RowProvider_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        let tree = self.tree.borrow();
        let popup = tree.popup()?;
        let next = match direction {
            NavigateDirection_Parent => return Ok(list(&self.tree).into()),
            NavigateDirection_NextSibling => popup.sibling(&self.id, 1),
            NavigateDirection_PreviousSibling => popup.sibling(&self.id, -1),
            _ => None,
        };
        match next {
            Some(r) => Ok(row(&self.tree, &r.id).into()),
            None => none(),
        }
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        let n = {
            let tree = self.tree.borrow();
            tree.row(&self.id)?;
            *tree.runtime.get(&self.id).ok_or_else(gone)?
        };
        runtime_array(n)
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        let tree = self.tree.borrow();
        let (_, row) = tree.row(&self.id)?;
        // Scrolled out of view: an empty rectangle.
        Ok(row.rect.map(|r| tree.screen(r)).unwrap_or_default())
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        self.select()
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        Ok(list(&self.tree).into())
    }
}

impl ISelectionItemProvider_Impl for RowProvider_Impl {
    fn Select(&self) -> Result<()> {
        self.select()
    }

    /// One row at most is selected: adding is selecting while none is.
    fn AddToSelection(&self) -> Result<()> {
        let none_selected = self.tree.borrow().popup()?.selected.is_none();
        if self.selected()? {
            Ok(())
        } else if none_selected {
            self.select()
        } else {
            Err(invalid_operation())
        }
    }

    /// The mark moves with the keys and the pointer; it cannot be taken away.
    fn RemoveFromSelection(&self) -> Result<()> {
        if self.selected()? {
            Err(invalid_operation())
        } else {
            Ok(())
        }
    }

    fn IsSelected(&self) -> Result<BOOL> {
        Ok(self.selected()?.into())
    }

    fn SelectionContainer(&self) -> Result<IRawElementProviderSimple> {
        self.tree.borrow().popup()?;
        Ok(list(&self.tree).into())
    }
}

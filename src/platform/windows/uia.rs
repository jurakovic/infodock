//! The bar as a UI Automation provider (SPEC §14.4). `WM_GETOBJECT` answers with a root provider
//! for the bar window, a fragment root whose children are the sensors; both answer from the
//! snapshot the bar logic hands over after each frame ([`AccessibleBar`]), so they never wait
//! for the bar logic. Comparing each snapshot with the last raises the events screen readers
//! follow (focus, values, popups, structure). What a screen reader asks of a sensor (Invoke,
//! ExpandCollapse, SetFocus) is queued and the window is told with [`WM_UIA`], since patterns
//! must return at once. The providers use COM threading: UI Automation calls them on the bar
//! thread, which created them.

// The Win32 constants keep their names, also as match patterns.
#![allow(non_upper_case_globals)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::ManuallyDrop;
use std::rc::Rc;
use std::time::Instant;

use windows::Win32::Foundation::{
    E_OUTOFMEMORY, HWND, LPARAM, LRESULT, RECT, VARIANT_FALSE, VARIANT_TRUE, WPARAM,
};
use windows::Win32::System::Com::SAFEARRAY;
use windows::Win32::System::Ole::{SafeArrayCreateVector, SafeArrayDestroy, SafeArrayPutElement};
use windows::Win32::System::Variant::{
    VARENUM, VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_BOOL, VT_BSTR, VT_I4, VariantClear,
};
use windows::Win32::UI::Accessibility::{
    ExpandCollapseState, ExpandCollapseState_Collapsed, ExpandCollapseState_Expanded,
    ExpandCollapseState_LeafNode, IExpandCollapseProvider, IExpandCollapseProvider_Impl,
    IInvokeProvider, IInvokeProvider_Impl, IRawElementProviderFragment,
    IRawElementProviderFragment_Impl, IRawElementProviderFragmentRoot,
    IRawElementProviderFragmentRoot_Impl, IRawElementProviderSimple,
    IRawElementProviderSimple_Impl, IValueProvider, IValueProvider_Impl, NavigateDirection,
    NavigateDirection_FirstChild, NavigateDirection_LastChild, NavigateDirection_NextSibling,
    NavigateDirection_Parent, NavigateDirection_PreviousSibling, ProviderOptions,
    ProviderOptions_ServerSideProvider, ProviderOptions_UseComThreading,
    StructureChangeType_ChildrenInvalidated, UIA_AutomationFocusChangedEventId,
    UIA_AutomationIdPropertyId, UIA_ButtonControlTypeId, UIA_ControlTypePropertyId,
    UIA_E_ELEMENTNOTAVAILABLE, UIA_ExpandCollapseExpandCollapseStatePropertyId,
    UIA_ExpandCollapsePatternId, UIA_HasKeyboardFocusPropertyId, UIA_InvokePatternId,
    UIA_IsEnabledPropertyId, UIA_IsKeyboardFocusablePropertyId, UIA_NamePropertyId, UIA_PATTERN_ID,
    UIA_PROPERTY_ID, UIA_ToolBarControlTypeId, UIA_ValuePatternId, UIA_ValueValuePropertyId,
    UiaAppendRuntimeId, UiaClientsAreListening, UiaHostProviderFromHwnd, UiaRaiseAutomationEvent,
    UiaRaiseAutomationPropertyChangedEvent, UiaRaiseStructureChangedEvent, UiaRect,
    UiaReturnRawElementProvider,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, PostMessageW, WM_APP};
use windows::core::{BSTR, Error, HRESULT, IUnknown, IUnknownImpl, PCWSTR, Result, implement};

use crate::i18n::t;
use crate::platform::accessible::{self, AccessibleEvent, Announced};
use crate::platform::{AccessibleAction, AccessibleBar, RectF};

/// The bar's automation id; the sensors' are their ids.
const BAR_ID: &str = "InfodockBar";
/// Posted to the bar window when a screen reader asked something of a sensor; the window
/// hands the queued actions to the bar logic ([`Uia::take_actions`]).
pub(super) const WM_UIA: u32 = WM_APP + 3;
/// The first part of a window's runtime id (`UiaHwndRuntimeIdBase`), followed by its handle.
const HWND_RUNTIME_ID: i32 = 42;

/// What the providers answer from, shared by all of them.
type Shared = Rc<RefCell<Tree>>;

struct Tree {
    hwnd: HWND,
    bar: AccessibleBar,
    /// Each sensor's runtime id, given when it is first seen and kept, so a sensor keeps its
    /// element while it is moved or changes what it shows.
    runtime: HashMap<String, i32>,
    /// Which values were last announced, and when.
    announced: Announced,
    /// What screen readers asked, for the bar logic.
    actions: Vec<AccessibleAction>,
}

impl Tree {
    /// Queues `action` for the bar logic and tells the window.
    fn ask(&mut self, action: AccessibleAction) {
        if self.actions.is_empty() {
            // SAFETY: posting to the bar window, which lives as long as the process.
            let _ = unsafe { PostMessageW(Some(self.hwnd), WM_UIA, WPARAM(0), LPARAM(0)) };
        }
        self.actions.push(action);
    }

    fn runtime_id(&mut self, id: &str) -> i32 {
        let next = i32::try_from(self.runtime.len()).unwrap_or(i32::MAX - 1) + 1;
        *self.runtime.entry(id.to_owned()).or_insert(next)
    }

    /// The window's top-left corner (physical screen px) and its scale.
    fn origin(&self) -> (f64, f64, f64) {
        let mut window = RECT::default();
        // SAFETY: plain queries on the bar window.
        let scale = unsafe {
            let _ = GetWindowRect(self.hwnd, &mut window);
            GetDpiForWindow(self.hwnd).max(96)
        };
        (
            f64::from(window.left),
            f64::from(window.top),
            f64::from(scale) / 96.0,
        )
    }

    /// `rect` (logical px in the bar) in physical screen px.
    fn screen(&self, rect: RectF) -> UiaRect {
        let (x, y, scale) = self.origin();
        UiaRect {
            left: x + f64::from(rect.left) * scale,
            top: y + f64::from(rect.top) * scale,
            width: f64::from(rect.width()) * scale,
            height: f64::from(rect.height()) * scale,
        }
    }

    /// A point in physical screen px as logical px in the bar.
    fn logical(&self, sx: f64, sy: f64) -> (f32, f32) {
        let (x, y, scale) = self.origin();
        (((sx - x) / scale) as f32, ((sy - y) / scale) as f32)
    }
}

/// The bar window's UI Automation provider, created at the first `WM_GETOBJECT` that asks for
/// it.
pub(super) struct Uia {
    root: IRawElementProviderSimple,
    tree: Shared,
}

impl Uia {
    pub(super) fn new(hwnd: HWND) -> Self {
        let tree = Rc::new(RefCell::new(Tree {
            hwnd,
            bar: AccessibleBar::default(),
            runtime: HashMap::new(),
            announced: Announced::default(),
            actions: Vec::new(),
        }));
        let root = BarProvider {
            tree: Rc::clone(&tree),
        }
        .into();
        Self { root, tree }
    }

    /// Answers a `WM_GETOBJECT` for `UiaRootObjectId` with the root provider.
    pub(super) fn answer(&self, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let hwnd = self.tree.borrow().hwnd;
        // SAFETY: our own window and provider, on the window's thread.
        unsafe { UiaReturnRawElementProvider(hwnd, wparam, lparam, &self.root) }
    }

    /// Takes the bar logic's latest description of the bar and raises the events that tell
    /// listening screen readers what changed.
    pub(super) fn update(&self, bar: AccessibleBar) {
        let events = {
            let mut tree = self.tree.borrow_mut();
            for item in &bar.items {
                tree.runtime_id(&item.id);
            }
            let tree = &mut *tree;
            let events = accessible::events(&tree.bar, &bar, &mut tree.announced, Instant::now());
            tree.bar = bar;
            events
        };
        // SAFETY: plain query.
        if events.is_empty() || !unsafe { UiaClientsAreListening() }.as_bool() {
            return;
        }
        // Raised with the snapshot released: in-process clients may ask the providers at once.
        for event in events {
            if let Err(e) = self.raise(event) {
                tracing::debug!("a UI Automation event could not be raised: {e}");
            }
        }
    }

    fn raise(&self, event: AccessibleEvent) -> Result<()> {
        let simple = |id: &str| -> IRawElementProviderSimple {
            SensorProvider {
                tree: Rc::clone(&self.tree),
                id: id.to_owned(),
            }
            .into()
        };
        // SAFETY: our own providers; the variants are freed after the calls.
        unsafe {
            match event {
                AccessibleEvent::Structure => {
                    let hwnd = self.tree.borrow().hwnd;
                    let mut id = [HWND_RUNTIME_ID, hwnd.0 as isize as i32];
                    UiaRaiseStructureChangedEvent(
                        &self.root,
                        StructureChangeType_ChildrenInvalidated,
                        id.as_mut_ptr(),
                        id.len() as i32,
                    )
                }
                AccessibleEvent::Focus(id) => {
                    UiaRaiseAutomationEvent(&simple(&id), UIA_AutomationFocusChangedEventId)
                }
                AccessibleEvent::Value { id, old, new } => {
                    let (old, new) = (Owned(text(&old)), Owned(text(&new)));
                    UiaRaiseAutomationPropertyChangedEvent(
                        &simple(&id),
                        UIA_ValueValuePropertyId,
                        &old.0,
                        &new.0,
                    )
                }
                AccessibleEvent::Expanded { id, open } => {
                    let state = |open| int(expand_state(Some(open)).0);
                    UiaRaiseAutomationPropertyChangedEvent(
                        &simple(&id),
                        UIA_ExpandCollapseExpandCollapseStatePropertyId,
                        &state(!open),
                        &state(open),
                    )
                }
            }
        }
    }

    /// What screen readers asked since the last call.
    pub(super) fn take_actions(&self) -> Vec<AccessibleAction> {
        std::mem::take(&mut self.tree.borrow_mut().actions)
    }

    /// The window is going: UI Automation lets go of its providers.
    pub(super) fn release(&self) {
        let hwnd = self.tree.borrow().hwnd;
        // SAFETY: the documented call for a window that is being destroyed.
        unsafe {
            UiaReturnRawElementProvider(
                hwnd,
                WPARAM(0),
                LPARAM(0),
                None::<&IRawElementProviderSimple>,
            );
        }
    }
}

/// Returns no element (a null pointer with success), as UI Automation expects for "none".
fn none<T>() -> Result<T> {
    Err(Error::empty())
}

fn gone() -> Error {
    Error::from(HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32))
}

fn variant(vt: VARENUM, value: VARIANT_0_0_0) -> VARIANT {
    VARIANT {
        Anonymous: VARIANT_0 {
            Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                vt,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: value,
            }),
        },
    }
}

/// A string for UI Automation, which frees it.
fn text(value: &str) -> VARIANT {
    variant(
        VT_BSTR,
        VARIANT_0_0_0 {
            bstrVal: ManuallyDrop::new(BSTR::from(value)),
        },
    )
}

/// A variant this side owns, freed when dropped.
struct Owned(VARIANT);

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: a variant built here, freed once.
        let _ = unsafe { VariantClear(&mut self.0) };
    }
}

fn expand_state(popup: Option<bool>) -> ExpandCollapseState {
    match popup {
        Some(true) => ExpandCollapseState_Expanded,
        Some(false) => ExpandCollapseState_Collapsed,
        None => ExpandCollapseState_LeafNode,
    }
}

fn int(value: i32) -> VARIANT {
    variant(VT_I4, VARIANT_0_0_0 { lVal: value })
}

fn boolean(value: bool) -> VARIANT {
    let value = if value { VARIANT_TRUE } else { VARIANT_FALSE };
    variant(VT_BOOL, VARIANT_0_0_0 { boolVal: value })
}

/// A runtime id for UI Automation, which frees it: appended to the window's.
fn runtime_array(n: i32) -> Result<*mut SAFEARRAY> {
    // SAFETY: a new two-element vector of i32, filled by index; destroyed again on failure.
    unsafe {
        let array = SafeArrayCreateVector(VT_I4, 0, 2);
        if array.is_null() {
            return Err(E_OUTOFMEMORY.into());
        }
        for (index, value) in [UiaAppendRuntimeId as i32, n].iter().enumerate() {
            let index = index as i32;
            if let Err(e) = SafeArrayPutElement(array, &index, std::ptr::from_ref(value).cast()) {
                let _ = SafeArrayDestroy(array);
                return Err(e);
            }
        }
        Ok(array)
    }
}

fn sensor(tree: &Shared, id: &str) -> IRawElementProviderFragment {
    SensorProvider {
        tree: Rc::clone(tree),
        id: id.to_owned(),
    }
    .into()
}

fn bar(tree: &Shared) -> BarProvider {
    BarProvider {
        tree: Rc::clone(tree),
    }
}

/// The bar window: a tool bar whose children are the sensors. The window's own provider
/// (`UiaHostProviderFromHwnd`) adds its place, focus and parent.
#[implement(
    IRawElementProviderSimple,
    IRawElementProviderFragment,
    IRawElementProviderFragmentRoot
)]
struct BarProvider {
    tree: Shared,
}

impl IRawElementProviderSimple_Impl for BarProvider_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading)
    }

    fn GetPatternProvider(&self, _pattern: UIA_PATTERN_ID) -> Result<IUnknown> {
        none()
    }

    fn GetPropertyValue(&self, property: UIA_PROPERTY_ID) -> Result<VARIANT> {
        Ok(match property {
            UIA_ControlTypePropertyId => int(UIA_ToolBarControlTypeId.0),
            UIA_NamePropertyId => text(&t("app.name")),
            UIA_AutomationIdPropertyId => text(BAR_ID),
            _ => VARIANT::default(),
        })
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        let hwnd = self.tree.borrow().hwnd;
        // SAFETY: our own window.
        unsafe { UiaHostProviderFromHwnd(hwnd) }
    }
}

impl IRawElementProviderFragment_Impl for BarProvider_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        let tree = self.tree.borrow();
        let child = match direction {
            NavigateDirection_FirstChild => tree.bar.items.first(),
            NavigateDirection_LastChild => tree.bar.items.last(),
            // The window's parent and siblings come from its host provider.
            _ => None,
        };
        match child {
            Some(item) => Ok(sensor(&self.tree, &item.id)),
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

impl IRawElementProviderFragmentRoot_Impl for BarProvider_Impl {
    fn ElementProviderFromPoint(&self, x: f64, y: f64) -> Result<IRawElementProviderFragment> {
        let tree = self.tree.borrow();
        let (x, y) = tree.logical(x, y);
        match tree.bar.item_at(x, y) {
            Some(item) => Ok(sensor(&self.tree, &item.id)),
            None => Ok(self.to_interface()),
        }
    }

    fn GetFocus(&self) -> Result<IRawElementProviderFragment> {
        let tree = self.tree.borrow();
        match tree.bar.focused_item() {
            Some(item) => Ok(sensor(&self.tree, &item.id)),
            None => none(),
        }
    }
}

/// A sensor on the bar: a button named by its type, its value what it shows; it can be
/// clicked (Invoke) and, with a popup, expanded and collapsed.
#[implement(
    IRawElementProviderSimple,
    IRawElementProviderFragment,
    IValueProvider,
    IInvokeProvider,
    IExpandCollapseProvider
)]
struct SensorProvider {
    tree: Shared,
    id: String,
}

impl SensorProvider_Impl {
    /// Runs `f` on this sensor's item; the element is gone once the sensor is.
    fn with<T>(&self, f: impl FnOnce(&crate::platform::AccessibleItem, &Tree) -> T) -> Result<T> {
        let tree = self.tree.borrow();
        let item = tree.bar.item(&self.id).ok_or_else(gone)?;
        Ok(f(item, &tree))
    }

    /// Hands `action` on this sensor to the bar logic.
    fn ask(&self, action: AccessibleAction) -> Result<()> {
        let mut tree = self.tree.borrow_mut();
        tree.bar.item(&self.id).ok_or_else(gone)?;
        tree.ask(action);
        Ok(())
    }
}

impl IRawElementProviderSimple_Impl for SensorProvider_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading)
    }

    fn GetPatternProvider(&self, pattern: UIA_PATTERN_ID) -> Result<IUnknown> {
        let popup = self.with(|item, _| item.popup.is_some())?;
        match pattern {
            UIA_ValuePatternId => Ok(self.to_interface::<IValueProvider>().into()),
            UIA_InvokePatternId => Ok(self.to_interface::<IInvokeProvider>().into()),
            UIA_ExpandCollapsePatternId if popup => {
                Ok(self.to_interface::<IExpandCollapseProvider>().into())
            }
            _ => none(),
        }
    }

    fn GetPropertyValue(&self, property: UIA_PROPERTY_ID) -> Result<VARIANT> {
        self.with(|item, tree| match property {
            UIA_ControlTypePropertyId => int(UIA_ButtonControlTypeId.0),
            UIA_NamePropertyId => text(&item.name),
            UIA_AutomationIdPropertyId => text(&item.id),
            UIA_IsKeyboardFocusablePropertyId | UIA_IsEnabledPropertyId => boolean(true),
            UIA_HasKeyboardFocusPropertyId => {
                boolean(tree.bar.focused_item().is_some_and(|f| f.id == item.id))
            }
            _ => VARIANT::default(),
        })
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        none()
    }
}

impl IRawElementProviderFragment_Impl for SensorProvider_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        let tree = self.tree.borrow();
        let next = match direction {
            NavigateDirection_Parent => return Ok(bar(&self.tree).into()),
            NavigateDirection_NextSibling => tree.bar.sibling(&self.id, 1),
            NavigateDirection_PreviousSibling => tree.bar.sibling(&self.id, -1),
            _ => None,
        };
        match next {
            Some(item) => Ok(sensor(&self.tree, &item.id)),
            None => none(),
        }
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        let n = {
            let mut tree = self.tree.borrow_mut();
            tree.bar.item(&self.id).ok_or_else(gone)?;
            tree.runtime_id(&self.id)
        };
        runtime_array(n)
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        self.with(|item, tree| tree.screen(item.rect))
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        self.ask(AccessibleAction::Focus(self.id.clone()))
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        Ok(bar(&self.tree).into())
    }
}

impl IValueProvider_Impl for SensorProvider_Impl {
    fn SetValue(&self, _value: &PCWSTR) -> Result<()> {
        // Read-only (SPEC §14.4).
        Err(Error::from(windows::Win32::Foundation::E_ACCESSDENIED))
    }

    fn Value(&self) -> Result<BSTR> {
        self.with(|item, _| BSTR::from(item.value.as_str()))
    }

    fn IsReadOnly(&self) -> Result<windows::core::BOOL> {
        Ok(true.into())
    }
}

impl IInvokeProvider_Impl for SensorProvider_Impl {
    fn Invoke(&self) -> Result<()> {
        self.ask(AccessibleAction::Invoke(self.id.clone()))
    }
}

impl IExpandCollapseProvider_Impl for SensorProvider_Impl {
    fn Expand(&self) -> Result<()> {
        self.ask(AccessibleAction::Expand(self.id.clone()))
    }

    fn Collapse(&self) -> Result<()> {
        self.ask(AccessibleAction::Collapse(self.id.clone()))
    }

    fn ExpandCollapseState(&self) -> Result<ExpandCollapseState> {
        self.with(|item, _| expand_state(item.popup))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
    };

    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationExpandCollapsePattern,
        IUIAutomationInvokePattern, IUIAutomationValuePattern,
    };
    use windows::Win32::UI::WindowsAndMessaging::FindWindowW;

    use super::*;
    use crate::platform::{AccessibleItem, BarHandler, BarWindow, Canvas, SizeF, UiEvent};

    /// A bar logic that only describes two sensors, the first with a popup and the second with
    /// the focus ring, and keeps what screen readers ask.
    struct Described(Arc<Mutex<Vec<AccessibleAction>>>);

    impl BarHandler for Described {
        fn on_event(&mut self, _window: &dyn BarWindow, event: UiEvent) {
            if let UiEvent::Accessible(action) = event {
                self.0
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(action);
            }
        }
        fn paint(&mut self, _canvas: &mut dyn Canvas, _size: SizeF) {}
        fn accessible(&self) -> AccessibleBar {
            let item = |id: &str, name: &str, value: &str, left: f32| AccessibleItem {
                id: id.into(),
                name: name.into(),
                value: value.into(),
                rect: RectF::new(left, 0.0, left + 50.0, 28.0),
                popup: None,
            };
            AccessibleBar {
                items: vec![
                    AccessibleItem {
                        popup: Some(false),
                        ..item("s1", "Folder Menu", "Documents", 4.0)
                    },
                    item("s2", "Time", "12:34", 60.0),
                ],
                focused: true,
                focus: Some(1),
            }
        }
    }

    fn value(element: &IUIAutomationElement) -> String {
        // SAFETY: plain UI Automation client calls.
        unsafe {
            element
                .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                .and_then(|p| p.CurrentValue())
                .map(|v| v.to_string())
                .unwrap_or_default()
        }
    }

    /// Waits up to five seconds for the bar logic to receive `action`.
    fn received(seen: &Mutex<Vec<AccessibleAction>>, action: &AccessibleAction) -> bool {
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until {
            if seen
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains(action)
            {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    #[ignore = "needs a Windows desktop session"]
    fn ui_automation_walks_the_bars_sensors_and_clicks_them() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let handler_seen = Arc::clone(&seen);
        crate::platform::start_bar_thread(Box::new(move |_| Box::new(Described(handler_seen))))
            .expect("the bar thread");
        // SAFETY: plain UI Automation client calls on this test's thread.
        unsafe {
            let hwnd = FindWindowW(super::super::window::CLASS_NAME, None).expect("the bar");
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).expect("UIA");
            let bar = automation
                .ElementFromHandle(hwnd)
                .expect("the bar's element");
            assert_eq!(bar.CurrentName().expect("name").to_string(), "Infodock");
            assert_eq!(
                bar.CurrentControlType().expect("type"),
                UIA_ToolBarControlTypeId
            );
            assert_eq!(bar.CurrentAutomationId().expect("id").to_string(), BAR_ID);

            let walker = automation.ControlViewWalker().expect("walker");
            let first = walker.GetFirstChildElement(&bar).expect("the first sensor");
            assert_eq!(
                first.CurrentName().expect("name").to_string(),
                "Folder Menu"
            );
            assert_eq!(first.CurrentAutomationId().expect("id").to_string(), "s1");
            assert_eq!(
                first.CurrentControlType().expect("type"),
                UIA_ButtonControlTypeId
            );
            assert_eq!(value(&first), "Documents");
            assert!(!first.CurrentHasKeyboardFocus().expect("focus").as_bool());
            // Its place on the screen: the window's corner plus its logical place, scaled.
            let mut window = RECT::default();
            let _ = GetWindowRect(hwnd, &mut window);
            let scale = f64::from(GetDpiForWindow(hwnd).max(96)) / 96.0;
            let rect = first.CurrentBoundingRectangle().expect("rect");
            assert_eq!(rect.left, window.left + (4.0 * scale).round() as i32);
            assert_eq!(rect.right - rect.left, (50.0 * scale).round() as i32);
            let parent = walker.GetParentElement(&first).expect("parent");
            assert!(
                automation
                    .CompareElements(&parent, &bar)
                    .expect("compare")
                    .as_bool()
            );

            let second = walker.GetNextSiblingElement(&first).expect("the second");
            assert_eq!(second.CurrentName().expect("name").to_string(), "Time");
            assert_eq!(value(&second), "12:34");
            assert!(second.CurrentHasKeyboardFocus().expect("focus").as_bool());
            assert!(walker.GetNextSiblingElement(&second).is_err(), "no third");

            // Invoke and ExpandCollapse reach the bar logic.
            second
                .GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
                .and_then(|p| p.Invoke())
                .expect("invoked");
            assert!(received(&seen, &AccessibleAction::Invoke("s2".into())));
            let expand = first
                .GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(
                    UIA_ExpandCollapsePatternId,
                )
                .expect("a popup to expand");
            assert_eq!(
                expand.CurrentExpandCollapseState().expect("state"),
                ExpandCollapseState_Collapsed
            );
            expand.Expand().expect("expanded");
            assert!(received(&seen, &AccessibleAction::Expand("s1".into())));
            assert!(
                second
                    .GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(
                        UIA_ExpandCollapsePatternId
                    )
                    .is_err(),
                "no popup, no expanding"
            );
        }
    }
}

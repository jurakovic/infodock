//! The bar as a UI Automation provider (SPEC §14.4). `WM_GETOBJECT` answers with a root provider
//! for the bar window, a fragment root whose children are the sensors; both answer from the
//! snapshot the bar logic hands over after each frame ([`AccessibleBar`]), so they never wait
//! for the bar logic. The providers use COM threading: UI Automation calls them on the bar
//! thread, which created them.

// The Win32 constants keep their names, also as match patterns.
#![allow(non_upper_case_globals)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::ManuallyDrop;
use std::rc::Rc;

use windows::Win32::Foundation::{
    E_OUTOFMEMORY, HWND, LPARAM, LRESULT, RECT, VARIANT_FALSE, VARIANT_TRUE, WPARAM,
};
use windows::Win32::System::Com::SAFEARRAY;
use windows::Win32::System::Ole::{SafeArrayCreateVector, SafeArrayDestroy, SafeArrayPutElement};
use windows::Win32::System::Variant::{
    VARENUM, VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_BOOL, VT_BSTR, VT_I4,
};
use windows::Win32::UI::Accessibility::{
    IRawElementProviderFragment, IRawElementProviderFragment_Impl, IRawElementProviderFragmentRoot,
    IRawElementProviderFragmentRoot_Impl, IRawElementProviderSimple,
    IRawElementProviderSimple_Impl, IValueProvider, IValueProvider_Impl, NavigateDirection,
    NavigateDirection_FirstChild, NavigateDirection_LastChild, NavigateDirection_NextSibling,
    NavigateDirection_Parent, NavigateDirection_PreviousSibling, ProviderOptions,
    ProviderOptions_ServerSideProvider, ProviderOptions_UseComThreading,
    UIA_AutomationIdPropertyId, UIA_ButtonControlTypeId, UIA_ControlTypePropertyId,
    UIA_E_ELEMENTNOTAVAILABLE, UIA_HasKeyboardFocusPropertyId, UIA_IsEnabledPropertyId,
    UIA_IsKeyboardFocusablePropertyId, UIA_NamePropertyId, UIA_PATTERN_ID, UIA_PROPERTY_ID,
    UIA_ToolBarControlTypeId, UIA_ValuePatternId, UiaAppendRuntimeId, UiaHostProviderFromHwnd,
    UiaRect, UiaReturnRawElementProvider,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
use windows::core::{BSTR, Error, HRESULT, IUnknown, IUnknownImpl, PCWSTR, Result, implement};

use crate::i18n::t;
use crate::platform::{AccessibleBar, RectF};

/// The bar's automation id; the sensors' are their ids.
const BAR_ID: &str = "InfodockBar";

/// What the providers answer from, shared by all of them.
type Shared = Rc<RefCell<Tree>>;

struct Tree {
    hwnd: HWND,
    bar: AccessibleBar,
    /// Each sensor's runtime id, given when it is first seen and kept, so a sensor keeps its
    /// element while it is moved or changes what it shows.
    runtime: HashMap<String, i32>,
}

impl Tree {
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

    /// Takes the bar logic's latest description of the bar.
    pub(super) fn update(&self, bar: AccessibleBar) {
        let mut tree = self.tree.borrow_mut();
        for item in &bar.items {
            tree.runtime_id(&item.id);
        }
        tree.bar = bar;
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

/// A sensor on the bar: a button named by its type, its value what it shows.
#[implement(IRawElementProviderSimple, IRawElementProviderFragment, IValueProvider)]
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
}

impl IRawElementProviderSimple_Impl for SensorProvider_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading)
    }

    fn GetPatternProvider(&self, pattern: UIA_PATTERN_ID) -> Result<IUnknown> {
        match pattern {
            UIA_ValuePatternId => Ok(self.to_interface::<IValueProvider>().into()),
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
        Ok(())
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

#[cfg(test)]
mod tests {
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationValuePattern,
    };
    use windows::Win32::UI::WindowsAndMessaging::FindWindowW;

    use super::*;
    use crate::platform::{AccessibleItem, BarHandler, BarWindow, Canvas, SizeF, UiEvent};

    /// A bar logic that only describes two sensors, the second with the focus ring.
    struct Described;

    impl BarHandler for Described {
        fn on_event(&mut self, _window: &dyn BarWindow, _event: UiEvent) {}
        fn paint(&mut self, _canvas: &mut dyn Canvas, _size: SizeF) {}
        fn accessible(&self) -> AccessibleBar {
            let item = |id: &str, name: &str, value: &str, left: f32| AccessibleItem {
                id: id.into(),
                name: name.into(),
                value: value.into(),
                rect: RectF::new(left, 0.0, left + 50.0, 28.0),
            };
            AccessibleBar {
                items: vec![
                    item("s1", "CPU Graph", "37%", 4.0),
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

    #[test]
    #[ignore = "needs a Windows desktop session"]
    fn ui_automation_walks_the_bars_sensors() {
        crate::platform::start_bar_thread(Box::new(|_| Box::new(Described)))
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
            assert_eq!(first.CurrentName().expect("name").to_string(), "CPU Graph");
            assert_eq!(first.CurrentAutomationId().expect("id").to_string(), "s1");
            assert_eq!(
                first.CurrentControlType().expect("type"),
                UIA_ButtonControlTypeId
            );
            assert_eq!(value(&first), "37%");
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
        }
    }
}

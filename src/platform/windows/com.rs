//! COM for threads that call into the shell (SPEC §4.1): a single-threaded apartment, entered
//! once per thread and left when the thread ends.

use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
};

/// Whether this thread's `CoInitializeEx` succeeded, so its `CoUninitialize` is owed.
struct Apartment(bool);

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: balances the successful CoInitializeEx of this thread.
            unsafe { CoUninitialize() };
        }
    }
}

thread_local! {
    static APARTMENT: Apartment = Apartment(
        // SAFETY: per-thread initialisation, balanced when the thread ends. A thread that is
        // in an apartment already (the bar thread) gets S_FALSE, which is balanced too.
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }
            .is_ok(),
    );
}

/// Makes sure this thread is in a COM apartment (a single-threaded one unless it already is in
/// another).
pub fn enter() {
    APARTMENT.with(|_| {});
}

//! NFC Bulk URL Writer: batch NTAG213/215/216 URL programming.

#![no_main]
#![no_std]

extern crate flipperzero_rt;

use core::ffi::CStr;
use core::ffi::c_void;
use core::mem::size_of;
use core::ptr;

use flipperzero::gui::Gui;
use flipperzero_rt::{entry, manifest};
use flipperzero_sys as sys;

mod app;
mod nfc;
mod storage;
mod ui;

manifest!(
    name = "NFC Bulk URL Writer",
    app_version = 1,
    stack_size = 16384,
    has_icon = true,
    // See https://github.com/flipperzero-rs/flipperzero/blob/v0.11.0/docs/icons.md
    icon = "nfc-bulk-url-writer-10x10.icon",
);

entry!(main);

fn main(_args: Option<&CStr>) -> i32 {
    let gui = Gui::open();

    // The app is heap-allocated so its address stays stable for the view
    // callbacks and the NFC worker, and so its fixed buffers do not exhaust the
    // application stack.
    let app_ptr = unsafe { sys::malloc(size_of::<app::App>() as u32) as *mut app::App };
    if app_ptr.is_null() {
        return 1;
    }

    unsafe {
        ptr::write(app_ptr, app::App::new());
        (*app_ptr).initialize(&gui);
        // Run the dispatcher without holding a Rust borrow of the app across
        // the blocking call; view callbacks mutate the app through its pointer.
        sys::view_dispatcher_run((*app_ptr).dispatcher());
        (*app_ptr).shutdown();
        ptr::drop_in_place(app_ptr);
        sys::free(app_ptr as *mut c_void);
    }

    drop(gui);
    0
}

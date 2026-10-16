// SPDX-License-Identifier: GPL-3.0-or-later
//! No-op `DG_*` platform callbacks for the library's own unit-test binary. The engine code in
//! the library references them; no unit test starts the engine, but MSVC's linker resolves
//! every reference before dropping unused code, so the test binary needs definitions. The
//! game binary (`platform/`) and the headless tests (`tests/support`) provide the real ones.

use std::ffi::c_char;

#[no_mangle]
pub extern "C" fn DG_Init() {}
#[no_mangle]
pub extern "C" fn DG_DrawFrame() {}
#[no_mangle]
pub extern "C" fn DG_SleepMs(_ms: u32) {}
#[no_mangle]
pub extern "C" fn DG_GetTicksMs() -> u32 {
    0
}
#[no_mangle]
pub extern "C" fn DG_GetKey(_pressed: *mut i32, _doom_key: *mut u8) -> i32 {
    0
}
#[no_mangle]
pub extern "C" fn DG_GetMouse(_dx: *mut i32, _dy: *mut i32, _buttons: *mut u8) -> i32 {
    0
}
#[no_mangle]
pub extern "C" fn DG_SetWindowTitle(_title: *const c_char) {}

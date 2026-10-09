#![cfg(target_os = "windows")]

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

const PE_HEADER_POINTER_OFFSET: u64 = 0x3c;
const COFF_HEADER_SIZE: u64 = 24;
const OPTIONAL_HEADER_SUBSYSTEM_OFFSET: u64 = 68;
const WINDOWS_GUI_SUBSYSTEM: u16 = 2;
const LOAD_LIBRARY_AS_DATAFILE: u32 = 0x0000_0002;
const APP_ICON_RESOURCE_ID: usize = 1;
const GROUP_ICON_RESOURCE_TYPE: usize = 14;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryExW(file_name: *const u16, file: *mut (), flags: u32) -> *mut ();
    fn FindResourceW(module: *mut (), name: *const u16, resource_type: *const u16) -> *mut ();
    fn FreeLibrary(module: *mut ()) -> i32;
}

#[test]
fn application_binary_embeds_app_icon() {
    let executable_path = Path::new(env!("CARGO_BIN_EXE_textora"));
    let wide_path: Vec<u16> = executable_path.as_os_str().encode_wide().chain(Some(0)).collect();
    let module = unsafe {
        LoadLibraryExW(wide_path.as_ptr(), std::ptr::null_mut(), LOAD_LIBRARY_AS_DATAFILE)
    };
    assert!(!module.is_null(), "Windows must load the textora executable as a resource file");

    let icon_resource = unsafe {
        FindResourceW(
            module,
            APP_ICON_RESOURCE_ID as *const u16,
            GROUP_ICON_RESOURCE_TYPE as *const u16,
        )
    };
    unsafe { FreeLibrary(module) };
    assert!(!icon_resource.is_null(), "the Windows executable must embed the app icon");
}

#[test]
fn application_binary_uses_windows_gui_subsystem() {
    let mut executable = File::open(env!("CARGO_BIN_EXE_textora"))
        .expect("Cargo must build the textora executable for integration tests");
    executable
        .seek(SeekFrom::Start(PE_HEADER_POINTER_OFFSET))
        .expect("Windows executable must have a PE header pointer");

    let mut header_pointer = [0; 4];
    executable
        .read_exact(&mut header_pointer)
        .expect("Windows executable must contain a PE header pointer");
    let subsystem_offset = u64::from(u32::from_le_bytes(header_pointer))
        + COFF_HEADER_SIZE
        + OPTIONAL_HEADER_SUBSYSTEM_OFFSET;
    executable
        .seek(SeekFrom::Start(subsystem_offset))
        .expect("Windows executable must have a subsystem field");

    let mut subsystem = [0; 2];
    executable
        .read_exact(&mut subsystem)
        .expect("Windows executable must contain a subsystem field");
    assert_eq!(
        u16::from_le_bytes(subsystem),
        WINDOWS_GUI_SUBSYSTEM,
        "the Windows executable must not open a console on launch"
    );
}

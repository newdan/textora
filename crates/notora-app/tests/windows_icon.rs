#![cfg(target_os = "windows")]

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

const LOAD_LIBRARY_AS_DATAFILE: u32 = 0x0000_0002;
const APP_ICON_RESOURCE_ID: usize = 1;
const GROUP_ICON_RESOURCE_TYPE: usize = 14;
const ICON_RESOURCE_TYPE: usize = 3;
const ICO_HEADER_BYTES: usize = 6;
const ICO_ENTRY_BYTES: usize = 16;
const GROUP_ICON_ENTRY_BYTES: usize = 14;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryExW(file_name: *const u16, file: *mut (), flags: u32) -> *mut ();
    fn FindResourceW(module: *mut (), name: *const u16, resource_type: *const u16) -> *mut ();
    fn LoadResource(module: *mut (), resource: *mut ()) -> *mut ();
    fn LockResource(resource: *mut ()) -> *const u8;
    fn SizeofResource(module: *mut (), resource: *mut ()) -> u32;
    fn FreeLibrary(module: *mut ()) -> i32;
}

fn resource_bytes(module: *mut (), resource_id: usize, resource_type: usize) -> Vec<u8> {
    let resource =
        unsafe { FindResourceW(module, resource_id as *const u16, resource_type as *const u16) };
    assert!(!resource.is_null(), "notora executable must contain its icon resource");
    let loaded = unsafe { LoadResource(module, resource) };
    assert!(!loaded.is_null(), "icon resource must load");
    let length = unsafe { SizeofResource(module, resource) } as usize;
    let bytes = unsafe { LockResource(loaded) };
    assert!(!bytes.is_null(), "icon resource bytes must be accessible");
    unsafe { std::slice::from_raw_parts(bytes, length) }.to_vec()
}

fn largest_ico_image(icon_bytes: &[u8]) -> &[u8] {
    let image_count = u16::from_le_bytes([icon_bytes[4], icon_bytes[5]]) as usize;
    for index in 0..image_count {
        let entry = ICO_HEADER_BYTES + index * ICO_ENTRY_BYTES;
        if icon_bytes[entry] != 0 || icon_bytes[entry + 1] != 0 {
            continue;
        }
        let length =
            u32::from_le_bytes(icon_bytes[entry + 8..entry + 12].try_into().expect("ICO length"))
                as usize;
        let start =
            u32::from_le_bytes(icon_bytes[entry + 12..entry + 16].try_into().expect("ICO offset"))
                as usize;
        return &icon_bytes[start..start + length];
    }
    panic!("Notora ICO must contain a 256-pixel image");
}

#[test]
fn notora_binary_embeds_the_notora_app_icon() {
    let executable_path = Path::new(env!("CARGO_BIN_EXE_notora"));
    let wide_path: Vec<u16> = executable_path.as_os_str().encode_wide().chain(Some(0)).collect();
    let module = unsafe {
        LoadLibraryExW(wide_path.as_ptr(), std::ptr::null_mut(), LOAD_LIBRARY_AS_DATAFILE)
    };
    assert!(!module.is_null(), "Windows must load the notora executable as a resource file");

    let icon_group = resource_bytes(module, APP_ICON_RESOURCE_ID, GROUP_ICON_RESOURCE_TYPE);
    let image_count = u16::from_le_bytes([icon_group[4], icon_group[5]]) as usize;
    let largest_entry = (0..image_count)
        .map(|index| ICO_HEADER_BYTES + index * GROUP_ICON_ENTRY_BYTES)
        .find(|&entry| icon_group[entry] == 0 && icon_group[entry + 1] == 0)
        .expect("embedded icon must include a 256-pixel image");
    let image_id = u16::from_le_bytes(
        icon_group[largest_entry + 12..largest_entry + 14]
            .try_into()
            .expect("icon group entry must contain an image ID"),
    );
    let embedded_image = resource_bytes(module, usize::from(image_id), ICON_RESOURCE_TYPE);
    unsafe { FreeLibrary(module) };
    assert_eq!(
        embedded_image,
        largest_ico_image(include_bytes!("../../../assets/NotoraAppIcon.ico"))
    );
}

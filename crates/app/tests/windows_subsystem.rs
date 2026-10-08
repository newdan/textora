#![cfg(target_os = "windows")]

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

const PE_HEADER_POINTER_OFFSET: u64 = 0x3c;
const COFF_HEADER_SIZE: u64 = 24;
const OPTIONAL_HEADER_SUBSYSTEM_OFFSET: u64 = 68;
const WINDOWS_GUI_SUBSYSTEM: u16 = 2;

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

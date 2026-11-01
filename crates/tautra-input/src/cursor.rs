use windows::Win32::UI::WindowsAndMessaging::{
    CreateCursor, GetCursorPos, SetSystemCursor, SystemParametersInfoW, OCR_APPSTARTING, OCR_CROSS, OCR_HAND,
    OCR_HELP, OCR_IBEAM, OCR_NO, OCR_NORMAL, OCR_SIZEALL, OCR_SIZENESW, OCR_SIZENS, OCR_SIZENWSE,
    OCR_SIZEWE, OCR_UP, OCR_WAIT, SPI_SETCURSORS, SYSTEM_CURSOR_ID,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
};

const CURSOR_SIZE: i32 = 32;
const PLANE_BYTES: usize = (CURSOR_SIZE * CURSOR_SIZE / 8) as usize;

const CURSOR_IDS: [SYSTEM_CURSOR_ID; 14] = [
    OCR_NORMAL,
    OCR_IBEAM,
    OCR_WAIT,
    OCR_CROSS,
    OCR_UP,
    OCR_SIZENWSE,
    OCR_SIZENESW,
    OCR_SIZEWE,
    OCR_SIZENS,
    OCR_SIZEALL,
    OCR_NO,
    OCR_HAND,
    OCR_APPSTARTING,
    OCR_HELP,
];

pub fn hide() -> Result<(), String> {
    let and_plane = [0xFFu8; PLANE_BYTES];
    let xor_plane = [0u8; PLANE_BYTES];
    for id in CURSOR_IDS {
        let cursor = unsafe {
            CreateCursor(
                None,
                0,
                0,
                CURSOR_SIZE,
                CURSOR_SIZE,
                and_plane.as_ptr().cast(),
                xor_plane.as_ptr().cast(),
            )
        }
        .map_err(|e| e.message())?;
        unsafe { SetSystemCursor(cursor, id) }.map_err(|e| e.message())?;
    }
    Ok(())
}

pub fn restore() -> Result<(), String> {
    unsafe {
        SystemParametersInfoW(
            SPI_SETCURSORS,
            0,
            None,
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .map_err(|e| e.message())
}

pub fn position() -> Option<(i32, i32)> {
    let mut point = Default::default();
    unsafe { GetCursorPos(&mut point) }.ok()?;
    Some((point.x, point.y))
}

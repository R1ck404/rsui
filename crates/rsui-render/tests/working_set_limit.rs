//! The limit on how much can be in RAM. It's on the whole process, so this has a program to itself, and one test so
//! that nothing runs beside it.

use std::ffi::c_void;

use rsui_render::WorkingSetLimit;

const HARD_MAXIMUM_IS_ON: u32 = 0x4;
const MB: usize = 1024 * 1024;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess() -> *mut c_void;
    fn GetProcessWorkingSetSizeEx(
        process: *mut c_void,
        minimum: *mut usize,
        maximum: *mut usize,
        flags: *mut u32,
    ) -> i32;
}

/// The limit in force on this process, if there is one.
fn hard_maximum() -> Option<usize> {
    let (mut minimum, mut maximum, mut flags) = (0, 0, 0);
    // SAFETY: this only reads this process's limits, and the out parameters are valid.
    let read = unsafe {
        GetProcessWorkingSetSizeEx(
            GetCurrentProcess(),
            &raw mut minimum,
            &raw mut maximum,
            &raw mut flags,
        )
    };
    assert_ne!(read, 0, "the limits can be read");
    (flags & HARD_MAXIMUM_IS_ON != 0).then_some(maximum)
}

#[test]
fn a_limit_holds_for_as_long_as_it_lasts_and_then_is_gone() {
    assert_eq!(hard_maximum(), None, "nothing is limited to begin with");

    let limit = WorkingSetLimit::with_room(MB, 512 * MB);
    assert_eq!(
        hard_maximum(),
        Some(512 * MB),
        "the least allowed wins when there's less than that in RAM"
    );
    drop(limit);
    assert_eq!(
        hard_maximum(),
        None,
        "ending it lets the process grow again"
    );

    let limit = WorkingSetLimit::with_room(64 * MB, 0);
    let maximum = hard_maximum().expect("another limit can be set afterwards");
    assert!(
        maximum > 64 * MB,
        "the room is on top of what's in RAM, and the limit was {maximum}"
    );
    drop(limit);
    assert_eq!(hard_maximum(), None, "that one ends too");
}

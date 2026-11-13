//! zkshim: stands in for /res/lib/libzkgui.so. The vendor launcher `zkgui`
//! dlopen()s the path from EasyUI.cfg `startupLibPath`; glibc runs our
//! .init_array constructor during dlopen, and we replace the process with
//! deckd. zkgui's PID (service `zkswe`) stays alive from init's point of view.
//!
//! Tries /tmp/bin/deckd first (dry runs), then /res/bin/deckd (installed).
//! If both fail it returns and zkgui continues; it will then fail to find
//! the EasyUI entry points and exit, and zkdaemon's recovery takes over.

#![no_std]

use core::arch::asm;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

const SYS_EXECVE: usize = 11;

unsafe fn execve(path: *const u8, argv: *const *const u8, envp: *const *const u8) -> isize {
    let ret: isize;
    asm!(
        "svc 0",
        in("r7") SYS_EXECVE,
        inlateout("r0") path as usize => ret,
        in("r1") argv,
        in("r2") envp,
        options(nostack)
    );
    ret
}

static PATH_TMP: &[u8] = b"/tmp/bin/deckd\0";
static PATH_RES: &[u8] = b"/res/bin/deckd\0";
static ENV_PATH: &[u8] = b"PATH=/sbin:/bin:/tmp\0";
static ENV_LD: &[u8] = b"LD_LIBRARY_PATH=/tmp:/res/lib:/lib\0";

unsafe extern "C" fn start_deckd() {
    let envp: [*const u8; 3] = [ENV_PATH.as_ptr(), ENV_LD.as_ptr(), core::ptr::null()];
    for path in [PATH_TMP, PATH_RES] {
        let argv: [*const u8; 2] = [path.as_ptr(), core::ptr::null()];
        execve(path.as_ptr(), argv.as_ptr(), envp.as_ptr());
        // only returns on failure; try the next candidate
    }
}

#[used]
#[link_section = ".init_array"]
static INIT: unsafe extern "C" fn() = start_deckd;

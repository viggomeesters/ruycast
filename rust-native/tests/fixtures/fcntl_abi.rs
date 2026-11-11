//! Darwin variadic fcntl entry shared by the owned fault injectors.
use super::checked;
use libc::c_int;

// Darwin arm64 puts the first variadic argument on the stack. Intel uses rdx.
// Only the original fixture's commands read an argument; unknown commands never do.
#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
pub(super) unsafe extern "C" fn test_fcntl(_fd: c_int, _command: c_int) -> c_int {
    core::arch::naked_asm!(
        "cmp w1, {getpath}", "b.eq 2f",
        "cmp w1, {dupfd}", "b.eq 3f",
        "cmp w1, {dupclo}", "b.eq 3f",
        "cmp w1, {setfd}", "b.eq 3f",
        "cmp w1, {setfl}", "b.eq 3f",
        "cmp w1, {setown}", "b.eq 3f",
        "cmp w1, {nocache}", "b.eq 3f",
        "cmp w1, {rdahead}", "b.eq 3f",
        "cmp w1, {nosigpipe}", "b.eq 3f",
        "mov x2, xzr", "b {checked}",
        "2:", "ldr x2, [sp]", "b {checked}",
        "3:", "ldrsw x2, [sp]", "b {checked}",
        getpath = const libc::F_GETPATH, dupfd = const libc::F_DUPFD,
        dupclo = const libc::F_DUPFD_CLOEXEC, setfd = const libc::F_SETFD,
        setfl = const libc::F_SETFL, setown = const libc::F_SETOWN,
        nocache = const libc::F_NOCACHE, rdahead = const libc::F_RDAHEAD,
        nosigpipe = const libc::F_SETNOSIGPIPE, checked = sym checked,
    );
}

#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
pub(super) unsafe extern "C" fn test_fcntl(_fd: c_int, _command: c_int) -> c_int {
    core::arch::naked_asm!(
        "cmp esi, {getpath}", "je 2f",
        "cmp esi, {dupfd}", "je 2f",
        "cmp esi, {dupclo}", "je 2f",
        "cmp esi, {setfd}", "je 2f",
        "cmp esi, {setfl}", "je 2f",
        "cmp esi, {setown}", "je 2f",
        "cmp esi, {nocache}", "je 2f",
        "cmp esi, {rdahead}", "je 2f",
        "cmp esi, {nosigpipe}", "je 2f",
        "xor edx, edx", "2:", "jmp {checked}",
        getpath = const libc::F_GETPATH, dupfd = const libc::F_DUPFD,
        dupclo = const libc::F_DUPFD_CLOEXEC, setfd = const libc::F_SETFD,
        setfl = const libc::F_SETFL, setown = const libc::F_SETOWN,
        nocache = const libc::F_NOCACHE, rdahead = const libc::F_RDAHEAD,
        nosigpipe = const libc::F_SETNOSIGPIPE, checked = sym checked,
    );
}

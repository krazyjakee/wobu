fn main() {
    reserve_a_main_thread_stack_the_size_of_everyone_else_s();
    tauri_build::build()
}

/// Give the Windows main thread the 8 MiB Linux and macOS already give it.
///
/// Windows reserves 1 MiB for a main thread — the MSVC linker's default, written
/// into the PE header — where Linux reserves 8 MiB and macOS 8 MiB. That eighth
/// is not a rounding difference, it is a platform where code that has been fine
/// everywhere else overflows, and it is what #149 was: accepting a share ticket
/// died with `STATUS_STACK_OVERFLOW` (`0xc00000fd`) on Windows and nowhere else,
/// silently, because a stack overflow is not a panic and so writes no log line,
/// runs no hook, and takes the process with it.
///
/// The specific overflow is fixed at its source in `src/sync.rs` — a command's
/// future is built on this thread, and that one was 133 KB. This is the other
/// half: the reason a 133 KB future was fatal at all rather than merely large.
/// Every command runs on this thread, so leaving the asymmetry in place means
/// the next one to grow fails the same way, on one platform, with no diagnostic.
///
/// Reserve, not commit. Windows backs stack pages on first touch, so the cost of
/// asking for 8 MiB and using 200 KB is address space, of which a 64-bit process
/// has more than it can spend.
fn reserve_a_main_thread_stack_the_size_of_everyone_else_s() {
    // Read from the environment rather than `cfg!`: a build script runs on the
    // host, and this has to follow the target — CI cross-builds.
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if windows && msvc {
        // Binaries only. `/STACK` is meaningless for the cdylib and staticlib
        // this crate also produces, which have no main thread of their own.
        println!("cargo::rustc-link-arg-bins=/STACK:8388608");
    }
}

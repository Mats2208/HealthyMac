//! Keeps scanning from getting in the way of whatever the user is doing.
//!
//! A scan is millions of filesystem calls. Past a handful of threads they mostly contend inside the
//! kernel, so more threads add heat rather than speed. Running them at utility QoS lets macOS place
//! them on the efficiency cores, where they use far less power and never compete with the UI.

/// The most threads worth using for filesystem walks, measured on a 15-core M5 Pro: 16 threads took
/// as long as 8 and spent 20% more time in the kernel.
const WALK_THREADS: usize = 8;

/// Marks the calling thread as background-ish work for the scheduler.
pub fn lower_current_thread_priority() {
    // SAFETY: only affects the calling thread, and the arguments are valid for this call. It can
    // only fail on an invalid class, and running at normal priority would be harmless anyway.
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0);
    }
}

/// Sets up rayon's global pool for scanning. Call it once, first thing in `main`. `RAYON_NUM_THREADS`
/// still overrides the thread count.
pub fn configure_global_pool() {
    let mut builder =
        rayon::ThreadPoolBuilder::new().start_handler(|_| lower_current_thread_priority());
    if std::env::var_os("RAYON_NUM_THREADS").is_none() {
        builder = builder.num_threads(WALK_THREADS);
    }
    // Only fails if the pool already exists, in which case it keeps working as it was.
    let _ = builder.build_global();
}

use crate::Result;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn signal_stop(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::Relaxed);
}

pub fn install_interrupt_handler() -> Result<()> {
    #[cfg(unix)]
    // The handler only performs a lock-free atomic store. All I/O stays in normal code.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = signal_stop as *const () as libc::sighandler_t;
        libc::sigemptyset(&mut action.sa_mask);
        for signal in [libc::SIGINT, libc::SIGTERM] {
            if libc::sigaction(signal, &action, std::ptr::null_mut()) != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
    }
    Ok(())
}

/// Linux/Unix address-space ceiling for direct CLI runs. The supervisor can
/// additionally sample resident memory across the whole child process group.
pub fn limit_address_space(mib: u64) -> Result<()> {
    if !(32..=16384).contains(&mib) {
        return Err("address-space limit must be 32..=16384 MiB".into());
    }
    #[cfg(unix)]
    unsafe {
        let mut limit: libc::rlimit = std::mem::zeroed();
        if libc::getrlimit(libc::RLIMIT_AS, &mut limit) != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        limit.rlim_cur = limit.rlim_cur.min(mib * 1024 * 1024);
        if libc::setrlimit(libc::RLIMIT_AS, &limit) != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
    #[cfg(not(unix))]
    Err("address-space limits require Unix".into())
}

pub struct Budget {
    pub started: Instant,
    deadline: Instant,
    search_end: Instant,
    requested: Option<u64>,
    work: AtomicU64,
    cancelled: AtomicBool,
}

impl Budget {
    pub fn new(
        started: Instant,
        seconds: f64,
        reserve: f64,
        requested: Option<u64>,
    ) -> Result<Self> {
        if !seconds.is_finite() || !(0.001..=86400.0).contains(&seconds) {
            return Err("time limit must be between 0.001 and 86400 seconds".into());
        }
        let deadline = started + Duration::from_secs_f64(seconds);
        let search_end = deadline - Duration::from_secs_f64(reserve.min(seconds * 0.30));
        Ok(Self {
            started,
            deadline,
            search_end,
            requested,
            work: AtomicU64::new(0),
            cancelled: AtomicBool::new(false),
        })
    }

    pub fn elapsed(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }
    pub fn work(&self) -> u64 {
        self.work.load(Ordering::Relaxed)
    }
    pub fn interrupted(&self) -> bool {
        INTERRUPTED.load(Ordering::Relaxed)
    }
    pub fn deadline_reached(&self) -> bool {
        Instant::now() >= self.deadline
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn in_search_time(&self) -> bool {
        !self.interrupted()
            && !self.cancelled.load(Ordering::Relaxed)
            && Instant::now() < self.search_end
    }

    pub fn can_search(&self) -> bool {
        self.in_search_time() && self.requested.is_none_or(|limit| self.work() < limit)
    }

    pub fn claim(&self) -> bool {
        if !self.in_search_time() {
            return false;
        }
        self.work
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                if self.requested.is_none_or(|cap| used < cap) {
                    used.checked_add(1)
                } else {
                    None
                }
            })
            .is_ok()
    }

    pub fn check_preprocessing(&self) -> Result<()> {
        if self.in_search_time() {
            Ok(())
        } else {
            Err("time limit or interruption during preprocessing".into())
        }
    }

    pub fn termination(&self) -> &'static str {
        if self.interrupted() {
            "interrupted"
        } else if self.requested.is_some_and(|cap| self.work() >= cap) {
            "work-limit"
        } else if Instant::now() >= self.search_end {
            "time-limit"
        } else {
            "search-finished"
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Checkpoint<C> {
    pub version: u32,
    pub solver_version: String,
    pub case_path: std::path::PathBuf,
    pub case_sha256: String,
    pub config: C,
    pub config_sha256: String,
    pub snapshot: crate::layout::Snapshot,
    pub total_seconds: f64,
    pub total_proposals: u64,
    pub attempts: u64,
}

use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

const MIN_WINDOW: Duration = Duration::from_millis(500);
const MB: f32 = 1024.0 * 1024.0;

pub struct Usage {
    pub cpu_percent: Option<f32>,
    pub working_set_mb: Option<f32>,
    pub private_mb: Option<f32>,
}

static LAST: Mutex<Option<(Instant, u64)>> = Mutex::new(None);

fn ticks(t: FILETIME) -> u64 {
    ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64
}

fn cpu_ticks() -> Option<u64> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe { GetProcessTimes(GetCurrentProcess(), &mut creation, &mut exit, &mut kernel, &mut user) }.ok()?;
    Some(ticks(kernel) + ticks(user))
}

fn memory() -> Option<PROCESS_MEMORY_COUNTERS_EX> {
    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    unsafe {
        K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS,
            counters.cb,
        )
    }
    .as_bool()
    .then_some(counters)
}

fn take_last() -> Option<(Instant, u64)> {
    LAST.lock().ok().and_then(|g| *g)
}

fn store_last(at: Instant, cpu: u64) {
    if let Ok(mut g) = LAST.lock() {
        *g = Some((at, cpu));
    }
}

pub async fn sample() -> Usage {
    let start = match take_last() {
        Some((at, cpu)) if at.elapsed() >= MIN_WINDOW => Some((at, cpu)),
        _ => {
            let at = Instant::now();
            let cpu = cpu_ticks();
            tokio::time::sleep(MIN_WINDOW).await;
            cpu.map(|c| (at, c))
        }
    };
    let now = Instant::now();
    let cpu_now = cpu_ticks();
    if let Some(c) = cpu_now {
        store_last(now, c);
    }
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f64;
    let cpu_percent = start.zip(cpu_now).and_then(|((at, before), after)| {
        let wall = now.duration_since(at).as_secs_f64();
        (wall > 0.0).then(|| {
            let busy = after.saturating_sub(before) as f64 / 10_000_000.0;
            (busy / wall / cores * 100.0) as f32
        })
    });
    let mem = memory();
    Usage {
        cpu_percent,
        working_set_mb: mem.map(|m| m.WorkingSetSize as f32 / MB),
        private_mb: mem.map(|m| m.PrivateUsage as f32 / MB),
    }
}

//! Runtime-only allocation probe for JSONL benchmark cases.
//! Run separately from timing benchmarks: the atomic counters add overhead.
use oneq::jq::{
    CompileOptions, Session,
    vm::{InputMode, host::InputHost},
};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicUsize, Ordering::Relaxed},
};

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static CALLS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
fn allocated(size: usize) {
    CALLS.fetch_add(1, Relaxed);
    BYTES.fetch_add(size, Relaxed);
    let live = LIVE.fetch_add(size, Relaxed) + size;
    PEAK.fetch_max(live, Relaxed);
}
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            allocated(layout.size());
        }
        ptr
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            allocated(layout.size());
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Relaxed);
        unsafe { System.dealloc(ptr, layout) };
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let next = unsafe { System.realloc(ptr, layout, size) };
        if !next.is_null() {
            LIVE.fetch_sub(layout.size(), Relaxed);
            allocated(size);
        }
        next
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .expect("usage: runtime_alloc CASE.jq");
    let source = std::fs::read_to_string(&path)?;
    let data = std::fs::read_to_string(std::path::Path::new(&path).with_extension("jsonl"))?;
    let inputs = data
        .lines()
        .map(oneq::data::parse_json_str)
        .collect::<Result<Vec<_>, _>>()?;
    let mut session = Session::new();
    let entry = session.append(&source, CompileOptions::default())?;
    let mut host = InputHost::new(inputs.into_iter().map(Ok));
    let baseline = LIVE.load(Relaxed);
    PEAK.store(baseline, Relaxed);
    CALLS.store(0, Relaxed);
    BYTES.store(0, Relaxed);
    let mut outputs = 0;
    for value in session.run(entry, &mut host, InputMode::Host)? {
        std::hint::black_box(value?);
        outputs += 1;
    }
    let calls = CALLS.load(Relaxed);
    let bytes = BYTES.load(Relaxed);
    let peak = PEAK.load(Relaxed);
    println!(
        "outputs={outputs} allocations_and_reallocations={calls} requested_bytes={bytes} baseline_live_bytes={baseline} peak_live_bytes={peak} peak_extra_bytes={}",
        peak - baseline
    );
    Ok(())
}

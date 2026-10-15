//! A rule whose harness claims the opposite of what it decides, which the proof gate must refuse.
#![no_std]

/// Whether one more use fits below `limit`.
#[must_use]
pub const fn admits(limit: u32, used: u32) -> bool {
    used < limit
}

#[cfg(kani)]
#[kani::proof]
fn the_gate_refuses_a_false_claim() {
    let limit: u32 = kani::any();
    let used: u32 = kani::any();
    kani::assume(used >= limit);
    kani::assert(admits(limit, used), "NEGATIVE_FIXTURE_CONTROL");
}

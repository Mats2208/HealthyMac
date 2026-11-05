//! Will this model fit in unified memory?

use std::ffi::CString;

use crate::types::{MemoryBudget, ModelFit};

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

/// Bytes the runtime needs on top of the weights: context cache, tokenizer and scratch buffers. A
/// fixed floor for small models, about a tenth of the weights for large ones. This is an estimate:
/// a long context on a large model can use more.
pub fn estimated_memory(model_bytes: u64) -> u64 {
    model_bytes.saturating_add((model_bytes / 10).max(512 * MIB))
}

/// Estimates fit from the model's size, which is a good proxy for weights in memory.
///
/// - `Comfortable`: the estimate is at most 75% of the GPU limit.
/// - `Tight`: above that, up to the GPU limit.
/// - `CpuOnly`: above the GPU limit but within RAM.
/// - `TooLarge`: beyond RAM.
/// - `Unknown`: no size or no budget to compare with.
pub fn assess(model_bytes: u64, budget: &MemoryBudget) -> ModelFit {
    if model_bytes == 0 || budget.total_ram == 0 {
        return ModelFit::Unknown;
    }
    let gpu_limit = match budget.gpu_limit {
        0 => default_gpu_limit(budget.total_ram),
        limit => limit.min(budget.total_ram),
    };
    let needed = estimated_memory(model_bytes);
    if needed > budget.total_ram {
        ModelFit::TooLarge
    } else if needed > gpu_limit {
        ModelFit::CpuOnly
    } else if needed as u128 * 4 <= gpu_limit as u128 * 3 {
        ModelFit::Comfortable
    } else {
        ModelFit::Tight
    }
}

/// Reads this machine's RAM and GPU wired limit.
///
/// `iogpu.wired_limit_mb` wins when the user has set it. At its default of 0, the limit is what
/// Metal itself recommends, and the fractions in [`default_gpu_limit`] are only the fallback when
/// no Metal device can be created.
pub fn current_budget() -> MemoryBudget {
    let total_ram = sysctl_u64("hw.memsize").unwrap_or(0);
    let gpu_limit = match sysctl_u64("iogpu.wired_limit_mb") {
        Some(megabytes) if megabytes > 0 => megabytes.saturating_mul(MIB).min(total_ram),
        _ => {
            hm_core::gpu::recommended_working_set().unwrap_or_else(|| default_gpu_limit(total_ram))
        }
    };
    MemoryBudget {
        total_ram,
        gpu_limit,
    }
}

/// macOS's default GPU working set when `iogpu.wired_limit_mb` is 0: about 2/3 of RAM below 36 GiB
/// and about 3/4 from there up.
///
/// Measured rather than assumed where possible: an M5 Pro with 48 GiB on macOS 27 reports
/// 40 200 896 512 bytes, which is 78%, so the 3/4 here errs slightly low. The published values for
/// 32 GiB (2/3) and 36 GiB (3/4) are why the boundary sits at 36 GiB inclusive.
pub fn default_gpu_limit(total_ram: u64) -> u64 {
    let total = total_ram as u128;
    let limit = if total_ram < 36 * GIB {
        total * 2 / 3
    } else {
        total * 3 / 4
    };
    limit as u64
}

fn sysctl_u64(name: &str) -> Option<u64> {
    let name = CString::new(name).ok()?;
    let mut value = [0u8; 8];
    let mut length = value.len();
    // SAFETY: `name` is NUL-terminated, and `value` and `length` are live, correctly sized locals.
    let result = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            value.as_mut_ptr().cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 {
        return None;
    }
    // Little-endian, and some of these are 32-bit: the upper bytes are then still zero.
    (length <= value.len()).then(|| u64::from_le_bytes(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUDGET: MemoryBudget = MemoryBudget {
        total_ram: 48 * GIB,
        gpu_limit: 36 * GIB,
    };

    /// Model size whose estimate is exactly `needed`, for sizes where the 10% term dominates.
    fn model_needing(needed: u64) -> u64 {
        let mut model = needed / 11 * 10;
        while estimated_memory(model) < needed {
            model += 1;
        }
        assert_eq!(estimated_memory(model), needed);
        model
    }

    #[test]
    fn boundaries_between_each_verdict() {
        let round = |bytes: u64| bytes / 11 * 11; // So an exact model size exists for it.
        let comfortable = round(BUDGET.gpu_limit / 4 * 3);
        assert_eq!(
            assess(model_needing(comfortable), &BUDGET),
            ModelFit::Comfortable
        );
        assert_eq!(
            assess(model_needing(comfortable + 11), &BUDGET),
            ModelFit::Tight
        );

        let gpu = round(BUDGET.gpu_limit);
        assert_eq!(assess(model_needing(gpu), &BUDGET), ModelFit::Tight);
        assert_eq!(assess(model_needing(gpu + 11), &BUDGET), ModelFit::CpuOnly);

        let ram = round(BUDGET.total_ram);
        assert_eq!(assess(model_needing(ram), &BUDGET), ModelFit::CpuOnly);
        assert_eq!(assess(model_needing(ram + 11), &BUDGET), ModelFit::TooLarge);
    }

    #[test]
    fn small_models_still_count_the_overhead_floor() {
        let tiny = MemoryBudget {
            total_ram: GIB,
            gpu_limit: 600 * MIB,
        };
        // 100 MiB of weights but 612 MiB of need, which is over the GPU limit.
        assert_eq!(assess(100 * MIB, &tiny), ModelFit::CpuOnly);
    }

    #[test]
    fn nothing_to_compare_is_unknown() {
        assert_eq!(assess(0, &BUDGET), ModelFit::Unknown);
        assert_eq!(assess(GIB, &MemoryBudget::default()), ModelFit::Unknown);
    }

    #[test]
    fn a_missing_gpu_limit_uses_the_default_and_an_inflated_one_is_capped_at_ram() {
        let unset = MemoryBudget {
            total_ram: 48 * GIB,
            gpu_limit: 0,
        };
        assert_eq!(assess(32 * GIB, &unset), ModelFit::Tight);
        let inflated = MemoryBudget {
            total_ram: 48 * GIB,
            gpu_limit: 512 * GIB,
        };
        assert_eq!(assess(100 * GIB, &inflated), ModelFit::TooLarge);
    }

    #[test]
    fn huge_sizes_do_not_overflow() {
        assert_eq!(assess(u64::MAX, &BUDGET), ModelFit::TooLarge);
        assert_eq!(
            assess(
                u64::MAX / 2,
                &MemoryBudget {
                    total_ram: u64::MAX,
                    gpu_limit: u64::MAX
                }
            ),
            ModelFit::Comfortable
        );
    }

    #[test]
    fn default_gpu_limit_switches_at_36_gib() {
        assert_eq!(default_gpu_limit(32 * GIB), 32 * GIB * 2 / 3);
        assert_eq!(default_gpu_limit(36 * GIB), 27 * GIB);
        assert_eq!(default_gpu_limit(48 * GIB), 36 * GIB);
    }

    #[test]
    fn this_machine_reports_a_sane_budget() {
        let budget = current_budget();
        assert!(budget.total_ram >= GIB);
        assert!(budget.gpu_limit > 0 && budget.gpu_limit <= budget.total_ram);
    }
}

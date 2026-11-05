//! Local AI features: a model inventory across every runtime, duplicate detection and APFS
//! deduplication, GGUF metadata, memory fit analysis and live runtime status.

mod duplicates;
mod files;
pub mod fit;
pub mod gguf;
mod hashcache;
mod http;
pub mod inventory;
mod locate;
mod metadata;
pub mod runtime;
pub mod scanners;
mod stores;
#[cfg(test)]
mod test_support;
pub mod types;
mod usage;

pub use scanners::scanners;

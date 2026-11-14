//! Native pagination slicing helper updating domain telemetry.

use super::metrics::DomainMetrics;

/// Slices `items` cleanly using Ironic's [`ironic::Pagination`] `offset`/`limit` bounds
/// and returns `(page_items, total_count)`.
#[must_use]
pub fn slice_paginated<T: Clone>(items: &[T], offset: u64, limit: u64) -> (Vec<T>, u64) {
    DomainMetrics::global().paginated_queries_total.inc();
    let total = u64::try_from(items.len()).unwrap_or(u64::MAX);
    let start = usize::try_from(offset)
        .unwrap_or(usize::MAX)
        .min(items.len());
    let take = usize::try_from(limit).unwrap_or(usize::MAX);
    let end = start.saturating_add(take).min(items.len());
    (items[start..end].to_vec(), total)
}

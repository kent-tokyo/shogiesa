use std::collections::{BTreeMap, BinaryHeap, HashMap, HashSet};

use crate::hash::seeded_hash;
use crate::heap::{HeapEntry, push_bounded};
use crate::{BucketKey, GroupKey};

/// Rank + hash tie-break key for one bucket's bounded heap in [`group_aware_fill`].
type RankHashKey = (u64, u64);

/// Bounded top-K reservoir sample: keeps `count` items keyed by `seeded_hash(seed, key_fn(item))`,
/// restoring original input order in the result (an A-Res-style weighted reservoir sample,
/// streamed at O(count) memory instead of materializing the whole input).
pub fn reservoir_sample<R>(
    items: impl Iterator<Item = R>,
    count: usize,
    seed: u64,
    key_fn: impl Fn(&R) -> &str,
) -> (Vec<R>, usize) {
    let mut heap: BinaryHeap<HeapEntry<u64, R>> =
        BinaryHeap::with_capacity(count.saturating_add(1));
    let mut total = 0usize;
    for record in items {
        let key = seeded_hash(seed, key_fn(&record));
        let index = total;
        total += 1;
        push_bounded(&mut heap, count, HeapEntry { key, index, record });
    }
    let mut kept: Vec<HeapEntry<u64, R>> = heap.into_vec();
    kept.sort_by_key(|e| e.index);
    (kept.into_iter().map(|e| e.record).collect(), total)
}

/// Outcome of [`group_aware_fill`]: kept items (original input order) plus the diversity/quota
/// stats a caller typically wants to report (e.g. in a run manifest).
pub struct GroupAwareFillResult<R> {
    pub kept: Vec<R>,
    pub total: usize,
    pub quota_candidates: usize,
    pub bucket_not_in_quota: usize,
    pub distinct_groups_kept: usize,
    /// Largest fraction any single group contributed to any one output bucket, considering only
    /// buckets with >=2 kept items (a singleton bucket is trivially "100% one group" and would
    /// otherwise always pin this at 1.0, telling a reader nothing about whether the fill actually
    /// diversified any bucket that had a real choice to make). `None` if no bucket had >=2 kept.
    pub max_group_share_in_any_bucket: Option<f64>,
}

/// Group-aware bounded-heap quota fill: streams `items` once, assigns each a bucket via
/// `bucket_key_fn`, and -- among items whose bucket has a quota in `quotas` -- fills that
/// bucket's quota preferring group diversity (`group_key_fn`) over first-come-first-served.
///
/// Algorithm: a pre-tally of `(bucket, group) -> how many already seen` gives each item a rank
/// (its group's Nth occurrence in that bucket, 0-indexed); the item is then pushed onto that
/// bucket's bounded top-`quota` heap keyed on `(rank, seeded_hash(seed, group))`. Lower rank
/// always wins, so every group's *first* occurrence in a bucket outranks every group's *second*
/// occurrence, across all groups -- one group can never consume a whole bucket's quota while a
/// second group present in it is starved out. The hash only breaks ties within one rank value.
pub fn group_aware_fill<R>(
    items: impl Iterator<Item = R>,
    quotas: &BTreeMap<BucketKey, usize>,
    seed: u64,
    bucket_key_fn: impl Fn(&R) -> BucketKey,
    group_key_fn: impl Fn(&R) -> GroupKey,
) -> GroupAwareFillResult<R> {
    let mut total = 0usize;
    let mut bucket_not_in_quota = 0usize;
    let mut quota_candidates = 0usize;
    let mut group_rank: HashMap<(BucketKey, GroupKey), u64> = HashMap::new();
    let mut heaps: HashMap<BucketKey, BinaryHeap<HeapEntry<RankHashKey, R>>> = HashMap::new();

    for record in items {
        let index = total;
        total += 1;
        let bucket = bucket_key_fn(&record);
        let Some(&quota) = quotas.get(&bucket) else {
            bucket_not_in_quota += 1;
            continue;
        };
        quota_candidates += 1;

        let group = group_key_fn(&record);
        let counter = group_rank
            .entry((bucket.clone(), group.clone()))
            .or_insert(0);
        let rank = *counter;
        *counter += 1;

        let key = (rank, seeded_hash(seed, &group));
        push_bounded(
            heaps.entry(bucket).or_default(),
            quota,
            HeapEntry { key, index, record },
        );
    }

    let mut kept: Vec<HeapEntry<RankHashKey, R>> =
        heaps.into_values().flat_map(|h| h.into_vec()).collect();
    kept.sort_by_key(|e| e.index);

    let mut per_bucket_groups: HashMap<BucketKey, HashMap<GroupKey, usize>> = HashMap::new();
    for entry in &kept {
        let bucket = bucket_key_fn(&entry.record);
        let group = group_key_fn(&entry.record);
        *per_bucket_groups
            .entry(bucket)
            .or_default()
            .entry(group)
            .or_default() += 1;
    }
    let max_group_share_in_any_bucket = per_bucket_groups
        .values()
        .filter_map(|groups| {
            let bucket_total: usize = groups.values().sum();
            (bucket_total >= 2)
                .then(|| groups.values().copied().max().unwrap_or(0) as f64 / bucket_total as f64)
        })
        .fold(None::<f64>, |acc, share| {
            Some(acc.map_or(share, |a| a.max(share)))
        });
    let distinct_groups_kept = kept
        .iter()
        .map(|e| group_key_fn(&e.record))
        .collect::<HashSet<_>>()
        .len();

    GroupAwareFillResult {
        kept: kept.into_iter().map(|e| e.record).collect(),
        total,
        quota_candidates,
        bucket_not_in_quota,
        distinct_groups_kept,
        max_group_share_in_any_bucket,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservoir_sample_restores_input_order_and_is_deterministic() {
        let items: Vec<String> = (0..20).map(|n: u32| n.to_string()).collect();
        let (kept_a, total_a) = reservoir_sample(items.clone().into_iter(), 5, 7, |s| s.as_str());
        let (kept_b, total_b) = reservoir_sample(items.into_iter(), 5, 7, |s| s.as_str());
        assert_eq!(total_a, 20);
        assert_eq!(total_b, 20);
        assert_eq!(kept_a, kept_b);
        assert_eq!(kept_a.len(), 5);
        let indices: Vec<u32> = kept_a.iter().map(|s| s.parse().unwrap()).collect();
        let mut sorted = indices.clone();
        sorted.sort();
        assert_eq!(indices, sorted, "result should be in original input order");
    }

    #[test]
    fn group_aware_fill_does_not_let_one_group_starve_out_another() {
        // 10 items from "root-a", 1 item from "root-b", all in one bucket with quota 2.
        let mut items: Vec<(&str, &str)> = (0..10).map(|_| ("bucket", "root-a")).collect();
        items.push(("bucket", "root-b"));
        let mut quotas = BTreeMap::new();
        quotas.insert("bucket".to_string(), 2);

        let result = group_aware_fill(
            items.into_iter(),
            &quotas,
            1,
            |(bucket, _)| bucket.to_string(),
            |(_, group)| group.to_string(),
        );

        assert_eq!(result.kept.len(), 2);
        let groups: HashSet<&str> = result.kept.iter().map(|(_, g)| *g).collect();
        assert!(
            groups.contains("root-b"),
            "the sole root-b item must survive, not be starved out"
        );
    }

    #[test]
    fn group_aware_fill_reports_bucket_not_in_quota() {
        let items = vec![("known", 1), ("unknown", 2)];
        let mut quotas = BTreeMap::new();
        quotas.insert("known".to_string(), 10);

        let result = group_aware_fill(
            items.into_iter(),
            &quotas,
            0,
            |(bucket, _)| bucket.to_string(),
            |(_, group)| group.to_string(),
        );

        assert_eq!(result.total, 2);
        assert_eq!(result.quota_candidates, 1);
        assert_eq!(result.bucket_not_in_quota, 1);
        assert_eq!(result.kept.len(), 1);
    }
}

use std::collections::BinaryHeap;

/// One candidate in a bounded top-K stream. `key` carries every tie-break the equivalent
/// full-materialize-then-`sort_by` code would apply; `index` is always the final tiebreak
/// layer, reproducing `sort_by`'s stability -- which a heap has no notion of on its own, since
/// two records can otherwise agree on every field `key` compares. `record` is left generic (`R`)
/// rather than fixed to any one domain's row type, so this heap is reusable outside shogiesa.
pub struct HeapEntry<K: Ord, R> {
    pub key: K,
    pub index: usize,
    pub record: R,
}

impl<K: Ord, R> PartialEq for HeapEntry<K, R> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.index == other.index
    }
}
impl<K: Ord, R> Eq for HeapEntry<K, R> {}
impl<K: Ord, R> PartialOrd for HeapEntry<K, R> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<K: Ord, R> Ord for HeapEntry<K, R> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key
            .cmp(&other.key)
            .then_with(|| self.index.cmp(&other.index))
    }
}

/// Keeps the `count` smallest `HeapEntry`s seen so far -- the standard bounded-heap top-K
/// algorithm: push while under capacity, otherwise evict the current worst-kept entry if `entry`
/// ranks ahead of it. Provably identical final set (and, via `BinaryHeap::into_sorted_vec`,
/// identical order) to "collect everything, sort ascending by the same key, truncate" -- at
/// O(count) memory instead of O(n).
pub fn push_bounded<K: Ord, R>(
    heap: &mut BinaryHeap<HeapEntry<K, R>>,
    count: usize,
    entry: HeapEntry<K, R>,
) {
    if heap.len() < count {
        heap.push(entry);
    } else if heap.peek().is_some_and(|worst| entry.cmp(worst).is_lt()) {
        heap.pop();
        heap.push(entry);
    }
}

/// `f32` wrapper with a total order (via `total_cmp`) for callers ranking by a plain fraction
/// (e.g. a 0..=1 quality score) that is never NaN in practice but has no `Ord` impl of its own.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TotalF32(pub f32);
impl Eq for TotalF32 {}
impl PartialOrd for TotalF32 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for TotalF32 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_bounded_keeps_k_smallest_keys() {
        let mut heap: BinaryHeap<HeapEntry<u32, &'static str>> = BinaryHeap::new();
        for (key, record) in [(5, "a"), (1, "b"), (9, "c"), (2, "d"), (7, "e")] {
            push_bounded(
                &mut heap,
                3,
                HeapEntry {
                    key,
                    index: key as usize,
                    record,
                },
            );
        }
        let mut kept: Vec<u32> = heap.into_vec().into_iter().map(|e| e.key).collect();
        kept.sort();
        assert_eq!(kept, vec![1, 2, 5]);
    }

    #[test]
    fn push_bounded_matches_sort_then_truncate() {
        let items: Vec<(u32, usize)> = vec![(3, 0), (1, 1), (4, 2), (1, 3), (5, 4), (9, 5), (2, 6)];
        let mut heap: BinaryHeap<HeapEntry<(u32, usize), usize>> = BinaryHeap::new();
        for &(key, index) in &items {
            push_bounded(
                &mut heap,
                4,
                HeapEntry {
                    key: (key, index),
                    index,
                    record: index,
                },
            );
        }
        let mut via_heap: Vec<usize> = heap
            .into_sorted_vec()
            .into_iter()
            .map(|e| e.record)
            .collect();

        let mut via_sort = items.clone();
        via_sort.sort_by_key(|&(key, index)| (key, index));
        via_sort.truncate(4);
        let via_sort: Vec<usize> = via_sort.into_iter().map(|(_, index)| index).collect();

        via_heap.sort();
        let mut via_sort_sorted = via_sort.clone();
        via_sort_sorted.sort();
        assert_eq!(via_heap, via_sort_sorted);
    }

    #[test]
    fn total_f32_orders_like_a_float_comparison() {
        let mut v = vec![TotalF32(0.5), TotalF32(0.1), TotalF32(0.9)];
        v.sort();
        assert_eq!(
            v.into_iter().map(|f| f.0).collect::<Vec<_>>(),
            vec![0.1, 0.5, 0.9]
        );
    }
}

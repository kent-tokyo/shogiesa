/// floor(value / bucket_size) * bucket_size. `bucket_size == 0` is a caller error, expected to be
/// checked and rejected at the caller's entry point, not deep inside this pure helper.
pub fn bucket_floor(value: u32, bucket_size: u32) -> u32 {
    (value / bucket_size) * bucket_size
}

pub fn mean_of(counts: &[usize]) -> f64 {
    if counts.is_empty() {
        0.0
    } else {
        counts.iter().sum::<usize>() as f64 / counts.len() as f64
    }
}

/// Coverage classification for one bucket in an enumerated bucket space -- e.g. "every
/// phase x side x eval-bucket combination", where a combination nobody observed still needs a
/// verdict (`Missing`), not just silent absence from a tally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BucketStatus {
    Missing,
    Under,
    Ok,
    Over,
}

impl std::fmt::Display for BucketStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            BucketStatus::Missing => "MISSING",
            BucketStatus::Under => "UNDER",
            BucketStatus::Ok => "OK",
            BucketStatus::Over => "OVER",
        })
    }
}

/// Ratio-to-mean classification: `count == 0` is always `Missing`; otherwise `Under`/`Over` when
/// `count` falls outside `[under_ratio, over_ratio] * mean`, else `Ok`.
pub fn classify_bucket(count: usize, mean: f64, under_ratio: f64, over_ratio: f64) -> BucketStatus {
    if count == 0 {
        return BucketStatus::Missing;
    }
    let count = count as f64;
    if count < under_ratio * mean {
        BucketStatus::Under
    } else if count > over_ratio * mean {
        BucketStatus::Over
    } else {
        BucketStatus::Ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_floor_rounds_down_to_multiple() {
        assert_eq!(bucket_floor(45, 10), 40);
        assert_eq!(bucket_floor(40, 10), 40);
        assert_eq!(bucket_floor(9, 10), 0);
    }

    #[test]
    fn mean_of_empty_is_zero() {
        assert_eq!(mean_of(&[]), 0.0);
    }

    #[test]
    fn mean_of_averages() {
        assert_eq!(mean_of(&[2, 4, 6]), 4.0);
    }

    #[test]
    fn classify_bucket_flags_missing_under_over_ok() {
        assert_eq!(classify_bucket(0, 10.0, 0.5, 2.0), BucketStatus::Missing);
        assert_eq!(classify_bucket(2, 10.0, 0.5, 2.0), BucketStatus::Under);
        assert_eq!(classify_bucket(25, 10.0, 0.5, 2.0), BucketStatus::Over);
        assert_eq!(classify_bucket(10, 10.0, 0.5, 2.0), BucketStatus::Ok);
    }
}

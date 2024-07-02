use near_primitives::types::{BlockHeight, ShardId};
use prometheus::core::Collector;
use prometheus::proto::Bucket;
use prometheus::proto::Histogram;

use crate::runtime::metrics::APPLYING_CHUNKS_TIME;
use crate::validate::GAS_LIMIT_ADJUSTMENT_FACTOR;

/// Determines how often `gas_limit` may be adjusted.
///
/// The `gas_limit` is not adjusted at every height to avoid too much overhead of related tasks like
/// metric collection and analysis.
pub(crate) const GAS_LIMIT_ADJUSTMENT_INTERVAL: u64 = 10;
/// At low load it is non-trivial to estimate if the node could handle more load, so we don't
/// adjust the `gas_limit`.
/// The value of 0.5 is chosen since upper bounds of available buckets are [..., 0.5, 1.0, ...].
const NOOP_CHUNK_APPLY_TIME: f64 = 0.5;
// TODO doc comments
const TARGET_CHUNK_APPLY_TIME: f64 = 1.0;
const THRESHOLD_NOOP: f64 = 0.5;
const THRESHOLD_INCREASE: f64 = 0.97;
const THRESHOLD_DECREASE: f64 = 0.94;

/// Assumes constant load close to what the node can handle. This requirement can be satisfied in
/// benchmark runs and allows simple logic to determine adjustments. In other scenarios more data
/// and a more elaborate algorithm are needed.
pub(crate) fn determine_new_gas_limit(
    gas_limit: u64,
    shard_id: ShardId,
    height: BlockHeight,
) -> u64 {
    if height % GAS_LIMIT_ADJUSTMENT_INTERVAL == 0 {
        // Avoid too frequent adjustments.
        return gas_limit;
    }

    let histogram = get_apply_chunk_time_histogram(shard_id);
    let bucket_noop = get_bucket(&histogram, NOOP_CHUNK_APPLY_TIME);
    // Looking at the bucket with 1.0 upper bound as 1 second is the max apply chunk time we hope to
    // see on mainnet.
    let bucket = get_bucket(&histogram, TARGET_CHUNK_APPLY_TIME);
    // TODO proper conversion to f64
    let ratio_noop =
        bucket_noop.get_cumulative_count() as f64 / histogram.get_sample_count() as f64;
    let ratio_within_target =
        bucket.get_cumulative_count() as f64 / histogram.get_sample_count() as f64;

    let mut new_gas_limit = gas_limit;
    if ratio_noop < THRESHOLD_NOOP {
        // Consider `gas_limit` adjustments only if there are enough high apply chunk times.
        if ratio_within_target >= THRESHOLD_INCREASE {
            new_gas_limit = gas_limit + gas_limit / GAS_LIMIT_ADJUSTMENT_FACTOR;
        } else if ratio_within_target <= THRESHOLD_DECREASE {
            new_gas_limit = gas_limit - gas_limit / GAS_LIMIT_ADJUSTMENT_FACTOR;
        }
    }
    new_gas_limit
}

// TODO avoid panics if this should be merged
fn get_bucket(histogram: &Histogram, upper_bound: f64) -> &Bucket {
    // Get the bucket with matching upper bound.
    // TODO search buckets instead of using a hardcoded index
    let idx = match upper_bound {
        // The 'magic' indices returned here are based on `try_create_histogram_vec`.
        x if x.abs() - 0.5 < f64::EPSILON => 5,
        x if x.abs() - 1.0 < f64::EPSILON => 6,
        x if x.abs() - 1.3 < f64::EPSILON => 7,
        _ => panic!("can't handle arbitrary upper bounds yet"),
    };
    let bucket = histogram.get_bucket().get(idx).expect("histogram should have more buckets");
    let got_upper_bound = bucket.get_upper_bound();
    assert!(
        got_upper_bound.abs() - upper_bound < f64::EPSILON,
        "got wrong bucket: want upper bound of {} but got {}",
        upper_bound,
        got_upper_bound
    );
    bucket
}

fn get_apply_chunk_time_histogram(shard_id: ShardId) -> Histogram {
    let hist = APPLYING_CHUNKS_TIME.with_label_values(&[&shard_id.to_string()]);
    let metric_family = hist.collect();
    assert_eq!(metric_family.len(), 1, "there should be one element in the MetricFamily");
    let metric = &metric_family[0].get_metric();
    assert_eq!(metric.len(), 1, "there should be one metric");
    metric[0].get_histogram().clone() // TODO avoid clone
}

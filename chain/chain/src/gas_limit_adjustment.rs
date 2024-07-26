use std::time::Duration;

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
/// Increasing the gas limit when it's too close to the target leads to overshooting.
const TARGET_BACKOFF: f64 = 0.05;
/// When there are no transactions, chunk apply times will be low regardless of a node's capacity.
/// Hence, in that case predicting if a node could handle more load is non-trivial.
///
/// To have a notion of a node being loaded, we pick an apply time smaller than
/// TARGET_CHUNK_APPLY_TIME.
const LOAD_INDICATION_APPLY_TIME: f64 = 0.5;
const THRESHOLD_NOOP: f64 = 0.5;
const THRESHOLD_INCREASE: f64 = 0.99;
const THRESHOLD_DECREASE: f64 = 0.99;

/// Assumes constant load close to what the node can handle. This requirement can be satisfied in
/// benchmark runs and allows simple logic to determine adjustments. In other scenarios more data
/// and a more elaborate algorithm are needed.
pub(crate) fn determine_new_gas_limit(
    gas_limit: u64,
    shard_id: ShardId,
    height: BlockHeight,
) -> u64 {
    if height % GAS_LIMIT_ADJUSTMENT_INTERVAL != 0 {
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
    println!("ratio noop: {ratio_noop}\tratio_within_target: {ratio_within_target}");

    let mut new_gas_limit = gas_limit;
    if ratio_within_target < THRESHOLD_DECREASE {
        // Too many chunk apply times exceed the target.
        new_gas_limit = gas_limit - gas_limit / GAS_LIMIT_ADJUSTMENT_FACTOR;
        println!("decreased gas_limit to {gas_limit}");
    } else if ratio_noop < THRESHOLD_NOOP {
        // Require sufficient amount of apply times to be out of noop-teritory for checking
        // `gas_limit` increas. Otherwise, if apply times are to short, making predictions about
        // node performance is more tricky.
        if ratio_within_target >= THRESHOLD_INCREASE {
            // Sufficiently many apply times within target, so let's increas the gas_limit.
            new_gas_limit = gas_limit + gas_limit / GAS_LIMIT_ADJUSTMENT_FACTOR;
            println!("increased gas_limit to {gas_limit}");
        }
    }

    new_gas_limit
}

/// Do throttling on caller side.
pub(crate) fn determine_new_gas_limit_2(
    gas_limit: u64,
    shard_id: ShardId,
    delayed_receipt_gas: u128,
) -> u64 {
    let histogram = get_apply_chunk_time_histogram(shard_id);
    let target_bucket = get_bucket(&histogram, TARGET_CHUNK_APPLY_TIME);
    // TODO proper conversion to f64
    let ratio_in_target =
        target_bucket.get_cumulative_count() as f64 / histogram.get_sample_count() as f64;

    if histogram.get_sample_count() % 50 == 0 {
        println!("ration_in_target: {ratio_in_target}\tdelayed_receipt_gas: {delayed_receipt_gas}");
    }

    let mut new_gas_limit = gas_limit;
    if ratio_in_target < THRESHOLD_DECREASE {
        // Too many chunk apply times exceed the target.
        new_gas_limit = gas_limit - gas_limit / GAS_LIMIT_ADJUSTMENT_FACTOR;
        println!("decreased gas_limit to {gas_limit}");
    } else if ratio_in_target > THRESHOLD_INCREASE && delayed_receipt_gas > 0 {
        // Chunk apply times are within the target, but still there are delayed receipts.
        // Take that as indication that the node could handle more, hence increase gas_limit.
        //
        // Looking at ratio_in_target alone is not sufficient. The reason for short short apply
        // times could be that there are few transactions.
        new_gas_limit = gas_limit + gas_limit / GAS_LIMIT_ADJUSTMENT_FACTOR;
        println!("increased gas_limit to {gas_limit}");
    }

    new_gas_limit
}

pub(crate) fn determine_new_gas_limit_3(
    gas_limit: u64,
    delayed_receipt_gas: u128,
    last_apply_time: Duration,
) -> u64 {
    let mut new_gas_limit = gas_limit;
    let last_apply_secs = last_apply_time.as_secs_f64();

    if last_apply_secs > TARGET_CHUNK_APPLY_TIME {
        // Apply times above the target are not acceptable, hence reduce `gas_limit`.
        new_gas_limit = gas_limit - gas_limit / GAS_LIMIT_ADJUSTMENT_FACTOR;
        println!("decreased gas_limit to {gas_limit}");
    } else if last_apply_secs > LOAD_INDICATION_APPLY_TIME
        && last_apply_secs <= TARGET_CHUNK_APPLY_TIME - TARGET_BACKOFF
    {
        // Without load it is hard to predict whether the node could handle more.
        // Therefore we consider increasing `gas_limit` only if there is some load.
        if delayed_receipt_gas > 0 {
            new_gas_limit = gas_limit + gas_limit / GAS_LIMIT_ADJUSTMENT_FACTOR;
            println!("increased gas_limit to {gas_limit}");
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
        x if x.abs() - 0.05 < f64::EPSILON => 2,
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

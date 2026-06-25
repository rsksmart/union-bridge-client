use metrics::{SharedString, Unit};
use metrics_util::debugging::{DebugValue, Snapshotter};
use metrics_util::{CompositeKey, MetricKind};

pub(crate) type DebugMetric = (CompositeKey, Option<Unit>, Option<SharedString>, DebugValue);

#[must_use]
pub(crate) fn snapshot(snapshotter: &Snapshotter) -> Vec<DebugMetric> {
    snapshotter.snapshot().into_vec()
}

#[must_use]
pub(crate) fn counter_value(metrics: &[DebugMetric], name: &str, labels: &[(&str, &str)]) -> u64 {
    metrics
        .iter()
        .find_map(|(key, _, _, value)| {
            let metric_key = key.key();
            let labels_match = labels.iter().all(|(label_key, label_value)| {
                metric_key
                    .labels()
                    .any(|label| label.key() == *label_key && label.value() == *label_value)
            });
            if key.kind() == MetricKind::Counter && metric_key.name() == name && labels_match {
                match value {
                    DebugValue::Counter(value) => Some(*value),
                    _ => None,
                }
            } else {
                None
            }
        })
        .unwrap_or(0)
}

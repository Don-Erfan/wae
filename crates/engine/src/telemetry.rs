use std::time::{Duration, Instant};

use crate::AnalysisTimings;

/// Internal phase accounting for one pipeline execution. Durations never overlap; everything
/// outside a named phase is reported as orchestration so the phase sum reconciles with total.
#[derive(Default)]
pub(crate) struct PipelineTelemetry {
    pub(crate) discovery: Duration,
    pub(crate) classification: Duration,
    pub(crate) parsing: Duration,
    pub(crate) resolution: Duration,
    pub(crate) graph_build: Duration,
    pub(crate) rule_evaluation: Duration,
    pub(crate) cache: Duration,
    pub(crate) reporting: Duration,
}

impl PipelineTelemetry {
    pub(crate) fn measure<T>(bucket: &mut Duration, operation: impl FnOnce() -> T) -> T {
        let started = Instant::now();
        let output = operation();
        *bucket += started.elapsed();
        output
    }

    pub(crate) fn finish(self, total: Duration) -> AnalysisTimings {
        let discovery_ms = self.discovery.as_millis();
        let classification_ms = self.classification.as_millis();
        let parsing_ms = self.parsing.as_millis();
        let resolution_ms = self.resolution.as_millis();
        let graph_build_ms = self.graph_build.as_millis();
        let rule_evaluation_ms = self.rule_evaluation.as_millis();
        let cache_ms = self.cache.as_millis();
        let reporting_ms = self.reporting.as_millis();
        let total_ms = total.as_millis();
        let named_ms = discovery_ms
            + classification_ms
            + parsing_ms
            + resolution_ms
            + graph_build_ms
            + rule_evaluation_ms
            + cache_ms
            + reporting_ms;
        AnalysisTimings {
            discovery_ms,
            classification_ms,
            parsing_ms,
            resolution_ms,
            graph_build_ms,
            rule_evaluation_ms,
            cache_ms,
            reporting_ms,
            orchestration_ms: total_ms.saturating_sub(named_ms),
            total_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_phases_and_orchestration_reconcile_with_total() {
        let metrics = PipelineTelemetry {
            discovery: Duration::from_millis(2),
            parsing: Duration::from_millis(3),
            resolution: Duration::from_millis(5),
            ..PipelineTelemetry::default()
        }
        .finish(Duration::from_millis(13));
        assert_eq!(metrics.orchestration_ms, 3);
        assert_eq!(
            metrics.discovery_ms
                + metrics.classification_ms
                + metrics.parsing_ms
                + metrics.resolution_ms
                + metrics.graph_build_ms
                + metrics.rule_evaluation_ms
                + metrics.cache_ms
                + metrics.reporting_ms
                + metrics.orchestration_ms,
            metrics.total_ms
        );
    }
}

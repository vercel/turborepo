use anyhow::Result;
use turbo_tasks::{emit, primitives::StringVc, ValueToString, ValueToStringVc};

#[turbo_tasks::value_trait]
pub trait NextTelemetry {
    fn event_name(&self) -> StringVc;
}

impl NextTelemetryVc {
    pub fn emit(self) {
        emit(self);
    }

    pub async fn peek_telemetries_with_path<T: turbo_tasks::CollectiblesSource + Copy>(
        source: T,
    ) -> Result<CapturedTelemetryVc> {
        Ok(CapturedTelemetryVc::cell(CapturedTelemetry {
            telemetry: source.peek_collectibles().strongly_consistent().await?,
        }))
    }
}

/// A struct represent telemetry event for feature usage,
/// referred as `importing` a certain module.
#[turbo_tasks::value(shared)]
pub struct ModuleFeatureTelemetry {
    pub event_name: String,
    pub feature_name: String,
    pub invocation_count: usize,
}

impl ModuleFeatureTelemetryVc {
    pub fn new(name: String, feature: String, invocation_count: usize) -> Self {
        Self::cell(ModuleFeatureTelemetry {
            event_name: name,
            feature_name: feature,
            invocation_count,
        })
    }
}

#[turbo_tasks::value_impl]
impl ValueToString for ModuleFeatureTelemetry {
    #[turbo_tasks::function]
    fn to_string(&self) -> StringVc {
        StringVc::cell(format!("{},{}", self.event_name, self.feature_name))
    }
}

#[turbo_tasks::value_impl]
impl NextTelemetry for ModuleFeatureTelemetry {
    #[turbo_tasks::function]
    async fn event_name(&self) -> Result<StringVc> {
        Ok(StringVc::cell(self.event_name.clone()))
    }
}

#[derive(Debug)]
#[turbo_tasks::value]
pub struct CapturedTelemetry {
    pub telemetry: auto_hash_map::AutoSet<NextTelemetryVc>,
}

#[turbo_tasks::value_trait]
pub trait TelemetryReporter {
    fn report_issues(
        &self,
        issues: turbo_tasks::TransientInstance<turbo_tasks::ReadRef<CapturedTelemetry>>,
        source: turbo_tasks::TransientValue<turbo_tasks::RawVc>,
    ) -> turbo_tasks::primitives::BoolVc;
}

pub trait TelemetryReporterProvider: Send + Sync + 'static {
    fn get_telemetry_reporter(&self) -> TelemetryReporterVc;
}

impl<T> TelemetryReporterProvider for T
where
    T: Fn() -> TelemetryReporterVc + Send + Sync + Clone + 'static,
{
    fn get_telemetry_reporter(&self) -> TelemetryReporterVc {
        self()
    }
}

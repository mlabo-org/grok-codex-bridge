use std::error::Error;
use std::fmt;
use std::path::PathBuf;

use crate::launchd::{
    LaunchAgentSpec, LaunchdError, ServiceStatus, service_install, service_status,
    service_uninstall,
};
use crate::lifecycle::{
    LifecycleError, PickerInstallReceipt, PickerInstallRequest, install_picker, uninstall_picker,
};

pub struct PickerActivationRequest {
    pub picker: PickerInstallRequest,
    pub launch_agent: LaunchAgentSpec,
    pub launch_agent_path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PickerActivationReceipt {
    pub picker: PickerInstallReceipt,
    pub prior_service_loaded: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PickerActivationStage {
    InspectPriorService,
    StopPriorService,
    PublishPicker,
    StartPublishedService,
    VerifyPublishedService,
}

impl fmt::Display for PickerActivationStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::InspectPriorService => "prior service inspection",
            Self::StopPriorService => "prior service stop",
            Self::PublishPicker => "picker publication",
            Self::StartPublishedService => "published service start",
            Self::VerifyPublishedService => "published service verification",
        };
        formatter.write_str(value)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PickerActivationOperationError {
    #[error(transparent)]
    Launchd(#[from] LaunchdError),
    #[error(transparent)]
    Lifecycle(#[from] LifecycleError),
    #[error("service status was {status:?}, not the required state")]
    UnexpectedServiceStatus { status: ServiceStatus },
    #[cfg(test)]
    #[error("injected picker activation failure: {0}")]
    Injected(&'static str),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PickerActivationRollbackStep {
    StopPublishedService,
    RemovePublishedPicker,
    RestorePriorService,
}

impl fmt::Display for PickerActivationRollbackStep {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::StopPublishedService => "stop published service",
            Self::RemovePublishedPicker => "remove published picker",
            Self::RestorePriorService => "restore prior service",
        };
        formatter.write_str(value)
    }
}

#[derive(Debug)]
pub struct PickerActivationRollbackFailure {
    pub step: PickerActivationRollbackStep,
    pub source: PickerActivationOperationError,
}

#[derive(Debug)]
pub struct PickerActivationError {
    pub stage: PickerActivationStage,
    pub source: PickerActivationOperationError,
    pub rollback_failures: Vec<PickerActivationRollbackFailure>,
}

impl fmt::Display for PickerActivationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "picker activation failed during {}: {}",
            self.stage, self.source
        )?;
        if !self.rollback_failures.is_empty() {
            formatter.write_str("; rollback also failed at ")?;
            for (index, failure) in self.rollback_failures.iter().enumerate() {
                if index != 0 {
                    formatter.write_str(", ")?;
                }
                write!(formatter, "{}: {}", failure.step, failure.source)?;
            }
        }
        Ok(())
    }
}

impl Error for PickerActivationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

pub fn activate_picker(
    request: &PickerActivationRequest,
) -> Result<PickerActivationReceipt, PickerActivationError> {
    let mut operations = SystemPickerActivationOperations { request };
    activate_picker_with_operations(&mut operations)
}

trait PickerActivationOperations {
    fn service_status(&mut self) -> Result<ServiceStatus, PickerActivationOperationError>;
    fn stop_service(&mut self) -> Result<(), PickerActivationOperationError>;
    fn publish_picker(&mut self) -> Result<PickerInstallReceipt, PickerActivationOperationError>;
    fn start_service(&mut self) -> Result<(), PickerActivationOperationError>;
    fn remove_picker(&mut self) -> Result<(), PickerActivationOperationError>;
}

struct SystemPickerActivationOperations<'a> {
    request: &'a PickerActivationRequest,
}

impl PickerActivationOperations for SystemPickerActivationOperations<'_> {
    fn service_status(&mut self) -> Result<ServiceStatus, PickerActivationOperationError> {
        service_status(&self.request.launch_agent).map_err(Into::into)
    }

    fn stop_service(&mut self) -> Result<(), PickerActivationOperationError> {
        service_uninstall(&self.request.launch_agent)
            .map(|_| ())
            .map_err(Into::into)
    }

    fn publish_picker(&mut self) -> Result<PickerInstallReceipt, PickerActivationOperationError> {
        install_picker(&self.request.picker).map_err(Into::into)
    }

    fn start_service(&mut self) -> Result<(), PickerActivationOperationError> {
        service_install(&self.request.launch_agent, &self.request.launch_agent_path)
            .map_err(Into::into)
    }

    fn remove_picker(&mut self) -> Result<(), PickerActivationOperationError> {
        uninstall_picker(
            &self.request.picker.install_root,
            &self.request.picker.codex_home,
        )
        .map(|_| ())
        .map_err(Into::into)
    }
}

fn activate_picker_with_operations<O: PickerActivationOperations>(
    operations: &mut O,
) -> Result<PickerActivationReceipt, PickerActivationError> {
    let prior_status = operations
        .service_status()
        .map_err(|source| PickerActivationError {
            stage: PickerActivationStage::InspectPriorService,
            source,
            rollback_failures: Vec::new(),
        })?;
    let prior_service_loaded = match prior_status {
        ServiceStatus::Loaded => true,
        ServiceStatus::NotLoaded => false,
        status @ ServiceStatus::Failed { .. } => {
            return Err(PickerActivationError {
                stage: PickerActivationStage::InspectPriorService,
                source: PickerActivationOperationError::UnexpectedServiceStatus { status },
                rollback_failures: Vec::new(),
            });
        }
    };

    if prior_service_loaded {
        operations
            .stop_service()
            .map_err(|source| PickerActivationError {
                stage: PickerActivationStage::StopPriorService,
                source,
                rollback_failures: Vec::new(),
            })?;
    }

    let picker = match operations.publish_picker() {
        Ok(receipt) => receipt,
        Err(source) => {
            let rollback_failures = restore_prior_service(operations, prior_service_loaded);
            return Err(PickerActivationError {
                stage: PickerActivationStage::PublishPicker,
                source,
                rollback_failures,
            });
        }
    };

    if let Err(source) = operations.start_service() {
        return Err(post_publication_failure(
            operations,
            prior_service_loaded,
            PickerActivationStage::StartPublishedService,
            source,
        ));
    }

    match operations.service_status() {
        Ok(ServiceStatus::Loaded) => Ok(PickerActivationReceipt {
            picker,
            prior_service_loaded,
        }),
        Ok(status) => Err(post_publication_failure(
            operations,
            prior_service_loaded,
            PickerActivationStage::VerifyPublishedService,
            PickerActivationOperationError::UnexpectedServiceStatus { status },
        )),
        Err(source) => Err(post_publication_failure(
            operations,
            prior_service_loaded,
            PickerActivationStage::VerifyPublishedService,
            source,
        )),
    }
}

fn post_publication_failure<O: PickerActivationOperations>(
    operations: &mut O,
    prior_service_loaded: bool,
    stage: PickerActivationStage,
    source: PickerActivationOperationError,
) -> PickerActivationError {
    let mut rollback_failures = Vec::new();
    if let Err(source) = operations.stop_service() {
        rollback_failures.push(PickerActivationRollbackFailure {
            step: PickerActivationRollbackStep::StopPublishedService,
            source,
        });
    }
    if let Err(source) = operations.remove_picker() {
        rollback_failures.push(PickerActivationRollbackFailure {
            step: PickerActivationRollbackStep::RemovePublishedPicker,
            source,
        });
    }
    rollback_failures.extend(restore_prior_service(operations, prior_service_loaded));
    PickerActivationError {
        stage,
        source,
        rollback_failures,
    }
}

fn restore_prior_service<O: PickerActivationOperations>(
    operations: &mut O,
    prior_service_loaded: bool,
) -> Vec<PickerActivationRollbackFailure> {
    if !prior_service_loaded {
        return Vec::new();
    }
    match operations.start_service() {
        Ok(()) => Vec::new(),
        Err(source) => vec![PickerActivationRollbackFailure {
            step: PickerActivationRollbackStep::RestorePriorService,
            source,
        }],
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Event {
        Status,
        Stop,
        Publish,
        Start,
        Remove,
    }

    struct FakeOperations {
        events: Vec<Event>,
        statuses: VecDeque<Result<ServiceStatus, PickerActivationOperationError>>,
        publish_error: Option<&'static str>,
        start_results: VecDeque<Result<(), PickerActivationOperationError>>,
        stop_error: Option<&'static str>,
        remove_error: Option<&'static str>,
    }

    impl FakeOperations {
        fn new(statuses: impl IntoIterator<Item = ServiceStatus>) -> Self {
            Self {
                events: Vec::new(),
                statuses: statuses.into_iter().map(Ok).collect(),
                publish_error: None,
                start_results: VecDeque::new(),
                stop_error: None,
                remove_error: None,
            }
        }
    }

    impl PickerActivationOperations for FakeOperations {
        fn service_status(&mut self) -> Result<ServiceStatus, PickerActivationOperationError> {
            self.events.push(Event::Status);
            self.statuses
                .pop_front()
                .expect("the test must provide every observed service status")
        }

        fn stop_service(&mut self) -> Result<(), PickerActivationOperationError> {
            self.events.push(Event::Stop);
            match self.stop_error.take() {
                Some(error) => Err(PickerActivationOperationError::Injected(error)),
                None => Ok(()),
            }
        }

        fn publish_picker(
            &mut self,
        ) -> Result<PickerInstallReceipt, PickerActivationOperationError> {
            self.events.push(Event::Publish);
            if let Some(error) = self.publish_error.take() {
                return Err(PickerActivationOperationError::Injected(error));
            }
            Ok(receipt())
        }

        fn start_service(&mut self) -> Result<(), PickerActivationOperationError> {
            self.events.push(Event::Start);
            self.start_results.pop_front().unwrap_or(Ok(()))
        }

        fn remove_picker(&mut self) -> Result<(), PickerActivationOperationError> {
            self.events.push(Event::Remove);
            match self.remove_error.take() {
                Some(error) => Err(PickerActivationOperationError::Injected(error)),
                None => Ok(()),
            }
        }
    }

    fn receipt() -> PickerInstallReceipt {
        PickerInstallReceipt {
            generated_catalog_path: PathBuf::from("/tmp/picker-models.json"),
            native_route_path: PathBuf::from("/tmp/picker-native-route.json"),
            managed_state_path: PathBuf::from("/tmp/picker-managed-state.json"),
            config_path: PathBuf::from("/tmp/config.toml"),
            native_model_count: 2,
            grok_model_count: 1,
        }
    }

    #[test]
    fn loaded_service_is_stopped_before_publication_then_started_and_verified() {
        let mut operations = FakeOperations::new([ServiceStatus::Loaded, ServiceStatus::Loaded]);

        let result = activate_picker_with_operations(&mut operations).unwrap();

        assert!(result.prior_service_loaded);
        assert_eq!(
            operations.events,
            [
                Event::Status,
                Event::Stop,
                Event::Publish,
                Event::Start,
                Event::Status,
            ]
        );
    }

    #[test]
    fn absent_service_is_published_then_started_and_verified() {
        let mut operations = FakeOperations::new([ServiceStatus::NotLoaded, ServiceStatus::Loaded]);

        let result = activate_picker_with_operations(&mut operations).unwrap();

        assert!(!result.prior_service_loaded);
        assert_eq!(
            operations.events,
            [Event::Status, Event::Publish, Event::Start, Event::Status]
        );
    }

    #[test]
    fn publication_failure_restores_a_previously_loaded_service() {
        let mut operations = FakeOperations::new([ServiceStatus::Loaded]);
        operations.publish_error = Some("publication");

        let error = activate_picker_with_operations(&mut operations).unwrap_err();

        assert_eq!(error.stage, PickerActivationStage::PublishPicker);
        assert!(error.rollback_failures.is_empty());
        assert_eq!(
            operations.events,
            [Event::Status, Event::Stop, Event::Publish, Event::Start]
        );
    }

    #[test]
    fn start_failure_removes_publication_then_restores_a_previously_loaded_service() {
        let mut operations = FakeOperations::new([ServiceStatus::Loaded]);
        operations
            .start_results
            .push_back(Err(PickerActivationOperationError::Injected("start")));
        operations.start_results.push_back(Ok(()));

        let error = activate_picker_with_operations(&mut operations).unwrap_err();

        assert_eq!(error.stage, PickerActivationStage::StartPublishedService);
        assert!(error.rollback_failures.is_empty());
        assert_eq!(
            operations.events,
            [
                Event::Status,
                Event::Stop,
                Event::Publish,
                Event::Start,
                Event::Stop,
                Event::Remove,
                Event::Start,
            ]
        );
    }

    #[test]
    fn status_failure_removes_publication_and_leaves_a_previously_absent_service_stopped() {
        let mut operations = FakeOperations::new([
            ServiceStatus::NotLoaded,
            ServiceStatus::Failed { exit_code: Some(3) },
        ]);

        let error = activate_picker_with_operations(&mut operations).unwrap_err();

        assert_eq!(error.stage, PickerActivationStage::VerifyPublishedService);
        assert!(error.rollback_failures.is_empty());
        assert_eq!(
            operations.events,
            [
                Event::Status,
                Event::Publish,
                Event::Start,
                Event::Status,
                Event::Stop,
                Event::Remove,
            ]
        );
    }
}

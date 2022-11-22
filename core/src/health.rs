//! Health control types
use serde::{Deserialize, Serialize};

use crate::error::SystemError;

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub enum RecoveryAction {
    Module(ModuleRecoveryAction),
    Partition(PartitionRecoveryAction),
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub enum ModuleRecoveryAction {
    Ignore,
    Shutdown,
    Reset,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub enum PartitionRecoveryAction {
    Idle,
    ColdStart,
    WarmStart,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PartitionHMTable {
    pub partition_init: RecoveryAction,
    pub segmentation: RecoveryAction,
    pub time_duration_exceeded: RecoveryAction,
    pub application_error: RecoveryAction,
    pub panic: RecoveryAction,
    pub floating_point_error: RecoveryAction,
    pub cgroup: RecoveryAction,
}

impl PartitionHMTable {
    pub fn try_action(&self, err: SystemError) -> Option<RecoveryAction> {
        match err {
            SystemError::PartitionInit => Some(self.partition_init),
            SystemError::Segmentation => Some(self.segmentation),
            SystemError::TimeDurationExceeded => Some(self.time_duration_exceeded),
            SystemError::ApplicationError => Some(self.application_error),
            SystemError::Panic => Some(self.panic),
            SystemError::FloatingPoint => Some(self.floating_point_error),
            SystemError::CGroup => Some(self.cgroup),
            _ => None,
        }
    }
}

impl Default for PartitionHMTable {
    fn default() -> Self {
        Self {
            partition_init: RecoveryAction::Module(ModuleRecoveryAction::Ignore),
            segmentation: RecoveryAction::Partition(PartitionRecoveryAction::WarmStart),
            //segmentation: RecoveryAction::Module(ModuleRecoveryAction::Reset),
            time_duration_exceeded: RecoveryAction::Module(ModuleRecoveryAction::Ignore),
            floating_point_error: RecoveryAction::Partition(PartitionRecoveryAction::WarmStart),
            panic: RecoveryAction::Partition(PartitionRecoveryAction::WarmStart),
            application_error: RecoveryAction::Partition(PartitionRecoveryAction::WarmStart),
            cgroup: RecoveryAction::Partition(PartitionRecoveryAction::WarmStart),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModuleInitHMTable {
    pub config: ModuleRecoveryAction,
    pub module_config: ModuleRecoveryAction,
    pub partition_config: ModuleRecoveryAction,
    pub partition_init: ModuleRecoveryAction,
    pub panic: ModuleRecoveryAction,
}

impl ModuleInitHMTable {
    pub fn try_action(&self, err: SystemError) -> Option<ModuleRecoveryAction> {
        match err {
            SystemError::Config => Some(self.config),
            SystemError::ModuleConfig => Some(self.module_config),
            SystemError::PartitionConfig => Some(self.partition_config),
            SystemError::PartitionInit => Some(self.partition_init),
            SystemError::Panic => Some(self.panic),
            _ => None,
        }
    }
}

impl Default for ModuleInitHMTable {
    fn default() -> Self {
        Self {
            config: ModuleRecoveryAction::Shutdown,
            module_config: ModuleRecoveryAction::Shutdown,
            partition_config: ModuleRecoveryAction::Shutdown,
            partition_init: ModuleRecoveryAction::Shutdown,
            panic: ModuleRecoveryAction::Shutdown,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModuleRunHMTable {
    pub partition_init: ModuleRecoveryAction,
    pub panic: ModuleRecoveryAction,
}

impl ModuleRunHMTable {
    pub fn try_action(&self, err: SystemError) -> Option<ModuleRecoveryAction> {
        match err {
            SystemError::PartitionInit => Some(self.partition_init),
            SystemError::Panic => Some(self.panic),
            _ => None,
        }
    }
}

impl Default for ModuleRunHMTable {
    fn default() -> Self {
        Self {
            partition_init: ModuleRecoveryAction::Shutdown,
            panic: ModuleRecoveryAction::Shutdown,
        }
    }
}

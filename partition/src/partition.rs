//TODO remove this
#![allow(dead_code)]

use crate::PART_NAME;

/// Static functions for within a partition
pub struct ApexLinuxPartition;

impl ApexLinuxPartition {
    pub fn get_partition_name() -> String {
        PART_NAME.clone()
    }
}

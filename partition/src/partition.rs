//TODO remove this
#![allow(dead_code)]

use crate::PART_NAME;

/// Static functions for within a partition
pub struct Partition;

impl Partition {
    pub fn get_partition_name() -> String {
        PART_NAME.clone()
    }
}

// Copyright 2019 PingCAP, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// See the License for the specific language governing permissions and
// limitations under the License.

//! Batch executor common structures.

use std::sync::Arc;

use tipb::schema::ColumnInfo;

use crate::coprocessor::codec::batch::LazyBatchColumnVec;
use crate::coprocessor::dag::expr::{EvalConfig, EvalWarnings};
use crate::coprocessor::Error;

/// The interface for pull-based executors. It is similar to the Volcano Iterator model, but
/// pulls data in batch and stores data by column.
pub trait BatchExecutor: Send {
    /// Pulls next several rows of data (stored by column).
    ///
    /// This function might return zero rows, which doesn't mean that there is no more result.
    /// See `is_drained` in `BatchExecuteResult`.
    fn next_batch(&mut self, expect_rows: usize) -> BatchExecuteResult;

    /// Collects statistics (including but not limited to metrics and execution summaries)
    /// accumulated during execution and prepares for next collection.
    ///
    /// The executor implementation must invoke this function for each children executor. However
    /// the invocation order of children executors is not stipulated.
    ///
    /// This function may be invoked several times during execution. For each invocation, it should
    /// not contain accumulated meta data in last invocation. Normally the invocation frequency of
    /// this function is less than `next_batch()`.
    fn collect_statistics(&mut self, destination: &mut BatchExecuteStatistics);
}

impl<T: BatchExecutor + ?Sized> BatchExecutor for Box<T> {
    fn next_batch(&mut self, expect_rows: usize) -> BatchExecuteResult {
        (**self).next_batch(expect_rows)
    }

    fn collect_statistics(&mut self, destination: &mut BatchExecuteStatistics) {
        (**self).collect_statistics(destination)
    }
}

/// A shared context for all batch executors.
///
/// It is both `Send` and `Sync`, allows concurrent access from different executors in future.
#[derive(Clone)]
pub struct BatchExecutorContext(Arc<BatchExecutorContextInner>);

impl BatchExecutorContext {
    pub fn new(columns_info: Vec<ColumnInfo>, config: EvalConfig) -> Self {
        let inner = BatchExecutorContextInner {
            columns_info,
            config,
        };
        BatchExecutorContext(Arc::new(inner))
    }

    /// Builds with a default config. Mainly used in tests.
    pub fn with_default_config(columns_info: Vec<ColumnInfo>) -> Self {
        Self::new(columns_info, EvalConfig::default())
    }
}

impl std::ops::Deref for BatchExecutorContext {
    type Target = BatchExecutorContextInner;

    fn deref(&self) -> &BatchExecutorContextInner {
        self.0.deref()
    }
}

impl crate::util::AssertSend for BatchExecutorContext {}

impl crate::util::AssertSync for BatchExecutorContext {}

pub struct BatchExecutorContextInner {
    pub columns_info: Vec<ColumnInfo>,

    // TODO: This is really a execution config, alhtough called eval config.
    pub config: EvalConfig,
}

/// Data to be flowed between parent and child executors' single `next_batch()` invocation.
///
/// Note: there are other data flow between executors, like metrics and output statistics.
/// However they are flowed at once, just before response, instead of each step during execution.
/// Hence they are not covered by this structure. See `BatchExecuteMetaData`.
///
/// TODO: Warnings should be flowed in each function call.
///
/// It is only `Send` but not `Sync` because executor returns its own data copy. However `Send`
/// enables executors to live in different threads.
///
/// It is designed to be used in new generation executors, i.e. executors support batch execution.
/// The old executors will not be refined to return this kind of result.
pub struct BatchExecuteResult {
    pub data: LazyBatchColumnVec,

    // TODO: It can be more general, e.g. `ExecuteWarnings` instead of `EvalWarnings`.
    // TODO: Should be recorded by row.
    pub warnings: EvalWarnings,

    pub is_drained: Result<bool, Error>,
}

/// Data to be flowed between parent and child executors at once during `collect_statistics()`
/// invocation.
///
/// Each batch executor aggregates and updates corresponding slots in this structure.
pub struct BatchExecuteStatistics {
    /// For each range given in the request, how many rows are scanned.
    pub scanned_rows_per_range: Vec<usize>,

    /// Scanning statistics for each CF during execution.
    pub cf_stats: crate::storage::Statistics,
    // TODO: Find somewhere to report exec count.
}

impl BatchExecuteStatistics {
    pub fn new(ranges: usize) -> Self {
        Self {
            scanned_rows_per_range: vec![0; ranges],
            cf_stats: crate::storage::Statistics::default(),
        }
    }

    pub fn clear(&mut self) {
        for item in self.scanned_rows_per_range.iter_mut() {
            *item = 0;
        }
        self.cf_stats = crate::storage::Statistics::default();
    }
}

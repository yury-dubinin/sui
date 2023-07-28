// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Immutable key/value store trait for storing/retrieving transactions, effects, and events
//! to/from a scalable.

use crate::key_value_store_metrics::KeyValueStoreMetrics;
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Instant;
use sui_types::digests::{TransactionDigest, TransactionEventsDigest};
use sui_types::effects::{TransactionEffects, TransactionEvents};
use sui_types::error::{SuiError, SuiResult};
use sui_types::transaction::Transaction;

pub struct TransactionKeyValueStore {
    store_name: &'static str,
    metrics: Arc<KeyValueStoreMetrics>,
    inner: Arc<dyn TransactionKeyValueStoreTrait + Send + Sync>,
}

impl TransactionKeyValueStore {
    pub fn new(
        store_name: &'static str,
        metrics: Arc<KeyValueStoreMetrics>,
        inner: Arc<dyn TransactionKeyValueStoreTrait + Send + Sync>,
    ) -> Self {
        Self {
            store_name,
            metrics,
            inner,
        }
    }

    /// Generic multi_get, allows implementors to get heterogenous values with a single round trip.
    pub async fn multi_get(
        &self,
        transactions: &[TransactionDigest],
        effects: &[TransactionDigest],
        events: &[TransactionEventsDigest],
    ) -> SuiResult<(
        Vec<Option<Transaction>>,
        Vec<Option<TransactionEffects>>,
        Vec<Option<TransactionEvents>>,
    )> {
        let start = Instant::now();
        let res = self.inner.multi_get(transactions, effects, events).await;
        let elapsed = start.elapsed();

        let num_txns = transactions.len() as u64;
        let num_effects = effects.len() as u64;
        let num_events = events.len() as u64;
        let total_keys = num_txns + num_effects + num_events;

        self.metrics
            .key_value_store_num_fetches_latency_ms
            .with_label_values(&[self.store_name])
            .observe(elapsed.as_millis() as u64);
        self.metrics
            .key_value_store_num_fetches_batch_size
            .with_label_values(&[self.store_name])
            .observe(total_keys);

        if let Ok(res) = &res {
            let txns_not_found = res.0.iter().filter(|v| v.is_none()).count() as u64;
            let effects_not_found = res.1.iter().filter(|v| v.is_none()).count() as u64;
            let events_not_found = res.2.iter().filter(|v| v.is_none()).count() as u64;

            if num_txns > 0 {
                self.metrics
                    .key_value_store_num_fetches_success
                    .with_label_values(&[self.store_name, "tx"])
                    .inc_by(num_txns);
            }
            if num_effects > 0 {
                self.metrics
                    .key_value_store_num_fetches_success
                    .with_label_values(&[self.store_name, "fx"])
                    .inc_by(num_effects);
            }
            if num_events > 0 {
                self.metrics
                    .key_value_store_num_fetches_success
                    .with_label_values(&[self.store_name, "events"])
                    .inc_by(num_events);
            }

            if txns_not_found > 0 {
                self.metrics
                    .key_value_store_num_fetches_not_found
                    .with_label_values(&[self.store_name, "tx"])
                    .inc_by(txns_not_found);
            }
            if effects_not_found > 0 {
                self.metrics
                    .key_value_store_num_fetches_not_found
                    .with_label_values(&[self.store_name, "fx"])
                    .inc_by(effects_not_found);
            }
            if events_not_found > 0 {
                self.metrics
                    .key_value_store_num_fetches_not_found
                    .with_label_values(&[self.store_name, "events"])
                    .inc_by(events_not_found);
            }
        } else {
            self.metrics
                .key_value_store_num_fetches_error
                .with_label_values(&[self.store_name, "tx"])
                .inc_by(num_txns);
            self.metrics
                .key_value_store_num_fetches_error
                .with_label_values(&[self.store_name, "fx"])
                .inc_by(num_effects);
            self.metrics
                .key_value_store_num_fetches_error
                .with_label_values(&[self.store_name, "events"])
                .inc_by(num_events);
        }

        res
    }

    pub async fn multi_get_tx(
        &self,
        keys: &[TransactionDigest],
    ) -> SuiResult<Vec<Option<Transaction>>> {
        self.multi_get(keys, &[], &[])
            .await
            .map(|(txns, _, _)| txns)
    }

    pub async fn multi_get_fx_by_tx_digest(
        &self,
        keys: &[TransactionDigest],
    ) -> SuiResult<Vec<Option<TransactionEffects>>> {
        self.multi_get(&[], keys, &[]).await.map(|(_, fx, _)| fx)
    }

    pub async fn multi_get_events(
        &self,
        keys: &[TransactionEventsDigest],
    ) -> SuiResult<Vec<Option<TransactionEvents>>> {
        self.multi_get(&[], &[], keys)
            .await
            .map(|(_, _, events)| events)
    }

    /// Convenience method for fetching single digest, and returning an error if it's not found.
    /// Prefer using multi_get_tx whenever possible.
    pub async fn get_tx(&self, digest: TransactionDigest) -> SuiResult<Transaction> {
        self.multi_get_tx(&[digest])
            .await?
            .into_iter()
            .next()
            .flatten()
            .ok_or(SuiError::TransactionNotFound { digest })
    }

    /// Convenience method for fetching single digest, and returning an error if it's not found.
    /// Prefer using multi_get_fx_by_tx_digest whenever possible.
    pub async fn get_fx_by_tx_digest(
        &self,
        digest: TransactionDigest,
    ) -> SuiResult<TransactionEffects> {
        self.multi_get_fx_by_tx_digest(&[digest])
            .await?
            .into_iter()
            .next()
            .flatten()
            .ok_or(SuiError::TransactionNotFound { digest })
    }

    /// Convenience method for fetching single digest, and returning an error if it's not found.
    /// Prefer using multi_get_events whenever possible.
    pub async fn get_events(
        &self,
        digest: TransactionEventsDigest,
    ) -> SuiResult<TransactionEvents> {
        self.multi_get_events(&[digest])
            .await?
            .into_iter()
            .next()
            .flatten()
            .ok_or(SuiError::TransactionEventsNotFound { digest })
    }
}

/// Immutable key/value store trait for storing/retrieving transactions, effects, and events.
/// Only defines multi_get/multi_put methods to discourage single key/value operations.
#[async_trait]
pub trait TransactionKeyValueStoreTrait {
    /// Generic multi_get, allows implementors to get heterogenous values with a single round trip.
    async fn multi_get(
        &self,
        transactions: &[TransactionDigest],
        effects: &[TransactionDigest],
        events: &[TransactionEventsDigest],
    ) -> SuiResult<(
        Vec<Option<Transaction>>,
        Vec<Option<TransactionEffects>>,
        Vec<Option<TransactionEvents>>,
    )>;
}

/// A TransactionKeyValueStoreTrait that falls back to a secondary store for any key for which the
/// primary store returns None.
///
/// Will be used to check the local rocksdb store, before falling back to a remote scalable store.
pub struct FallbackTransactionKVStore {
    primary: TransactionKeyValueStore,
    fallback: TransactionKeyValueStore,
}

impl FallbackTransactionKVStore {
    pub fn new_kv(
        primary: TransactionKeyValueStore,
        fallback: TransactionKeyValueStore,
        metrics: Arc<KeyValueStoreMetrics>,
        label: &'static str,
    ) -> TransactionKeyValueStore {
        let store = Arc::new(Self { primary, fallback });
        TransactionKeyValueStore::new(label, metrics, store)
    }
}

#[async_trait]
impl TransactionKeyValueStoreTrait for FallbackTransactionKVStore {
    async fn multi_get(
        &self,
        transactions: &[TransactionDigest],
        effects: &[TransactionDigest],
        events: &[TransactionEventsDigest],
    ) -> SuiResult<(
        Vec<Option<Transaction>>,
        Vec<Option<TransactionEffects>>,
        Vec<Option<TransactionEvents>>,
    )> {
        let mut res = self
            .primary
            .multi_get(transactions, effects, events)
            .await?;

        let (fallback_transactions, indices_transactions) = find_fallback(&res.0, transactions);
        let (fallback_effects, indices_effects) = find_fallback(&res.1, effects);
        let (fallback_events, indices_events) = find_fallback(&res.2, events);

        if fallback_transactions.is_empty()
            && fallback_effects.is_empty()
            && fallback_events.is_empty()
        {
            return Ok(res);
        }

        let secondary_res = self
            .fallback
            .multi_get(&fallback_transactions, &fallback_effects, &fallback_events)
            .await?;

        merge_res(&mut res.0, secondary_res.0, &indices_transactions);
        merge_res(&mut res.1, secondary_res.1, &indices_effects);
        merge_res(&mut res.2, secondary_res.2, &indices_events);

        Ok((res.0, res.1, res.2))
    }
}

fn find_fallback<T, K: Clone>(values: &[Option<T>], keys: &[K]) -> (Vec<K>, Vec<usize>) {
    let num_nones = values.iter().filter(|v| v.is_none()).count();
    let mut fallback_keys = Vec::with_capacity(num_nones);
    let mut fallback_indices = Vec::with_capacity(num_nones);
    for (i, value) in values.iter().enumerate() {
        if value.is_none() {
            fallback_keys.push(keys[i].clone());
            fallback_indices.push(i);
        }
    }
    (fallback_keys, fallback_indices)
}

fn merge_res<T>(values: &mut [Option<T>], fallback_values: Vec<Option<T>>, indices: &[usize]) {
    for (&index, fallback_value) in indices.iter().zip(fallback_values) {
        values[index] = fallback_value;
    }
}
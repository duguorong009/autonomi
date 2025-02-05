// Copyright 2024 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::{error::Result, node::Node};
use ant_evm::ProofOfPayment;
use ant_networking::{GetRecordCfg, Network, ResponseQuorum};
use ant_protocol::storage::DataTypes;
use ant_protocol::{
    messages::{Cmd, Query, QueryResponse, Request, Response},
    storage::ValidationType,
    NetworkAddress, PrettyPrintRecordKey,
};
use libp2p::{
    kad::{Record, RecordKey},
    PeerId,
};
use tokio::task::spawn;

impl Node {
    /// Sends _all_ record keys every interval to all peers within the REPLICATE_RANGE.
    pub(crate) fn try_interval_replication(network: Network) {
        network.trigger_interval_replication()
    }

    /// Cleanup unrelevant records if accumulated too many.
    pub(crate) fn trigger_irrelevant_record_cleanup(network: Network) {
        network.trigger_irrelevant_record_cleanup()
    }

    /// Get the Record from a peer or from the network without waiting.
    pub(crate) fn fetch_replication_keys_without_wait(
        &self,
        keys_to_fetch: Vec<(PeerId, RecordKey)>,
    ) -> Result<()> {
        for (holder, key) in keys_to_fetch {
            let node = self.clone();
            let requester = NetworkAddress::from_peer(self.network().peer_id());
            let _handle = spawn(async move {
                let pretty_key = PrettyPrintRecordKey::from(&key).into_owned();
                debug!("Fetching record {pretty_key:?} from node {holder:?}");
                let req = Request::Query(Query::GetReplicatedRecord {
                    requester,
                    key: NetworkAddress::from_record_key(&key),
                });
                let record_opt = if let Ok(resp) = node.network().send_request(req, holder).await {
                    match resp {
                        Response::Query(QueryResponse::GetReplicatedRecord(result)) => match result
                        {
                            Ok((_holder, record_content)) => Some(record_content),
                            Err(err) => {
                                debug!("Failed fetch record {pretty_key:?} from node {holder:?}, with error {err:?}");
                                None
                            }
                        },
                        other => {
                            debug!("Cannot fetch record {pretty_key:?} from node {holder:?}, with response {other:?}");
                            None
                        }
                    }
                } else {
                    None
                };

                let record = if let Some(record_content) = record_opt {
                    Record::new(key, record_content.to_vec())
                } else {
                    debug!(
                        "Can not fetch record {pretty_key:?} from node {holder:?}, fetching from the network"
                    );
                    let get_cfg = GetRecordCfg {
                        get_quorum: ResponseQuorum::One,
                        retry_strategy: Default::default(),
                        target_record: None,
                        expected_holders: Default::default(),
                    };
                    match node.network().get_record_from_network(key, &get_cfg).await {
                        Ok(record) => record,
                        Err(err) => {
                            error!("During replication fetch of {pretty_key:?}, failed in re-attempt of get from network {err:?}");
                            return;
                        }
                    }
                };

                debug!(
                    "Got Replication Record {pretty_key:?} from network, validating and storing it"
                );
                if let Err(err) = node.store_replicated_in_record(record).await {
                    error!("During store replication fetched {pretty_key:?}, got error {err:?}");
                } else {
                    debug!("Completed storing Replication Record {pretty_key:?} from network.");
                }
            });
        }
        Ok(())
    }

    /// Replicate a fresh record to its close group peers.
    /// This should not be triggered by a record we receive via replicaiton fetch
    pub(crate) fn replicate_valid_fresh_record(
        &self,
        paid_key: RecordKey,
        data_type: DataTypes,
        validation_type: ValidationType,
        payment: Option<ProofOfPayment>,
    ) {
        let network = self.network().clone();

        let _handle = spawn(async move {
            let start = std::time::Instant::now();
            let pretty_key = PrettyPrintRecordKey::from(&paid_key);

            // first we wait until our own network store can return the record
            // otherwise it may not be fully written yet
            let mut retry_count = 0;
            debug!("Checking we have successfully stored the fresh record {pretty_key:?} in the store before replicating");
            loop {
                let record = network.get_local_record(&paid_key).await.unwrap_or_else(|err| {
                    error!(
                            "Replicating fresh record {pretty_key:?} get_record_from_store errored: {err:?}"
                        );
                    None
                });

                if record.is_some() {
                    break;
                }

                if retry_count > 10 {
                    error!(
                        "Could not get record from store for replication: {pretty_key:?} after 10 retries"
                    );
                    return;
                }

                retry_count += 1;
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }

            debug!("Start replication of fresh record {pretty_key:?} from store");

            let data_addr = NetworkAddress::from_record_key(&paid_key);

            // If payment exists, only candidates are the payees.
            // Else get candidates from network.
            let replicate_candidates = match payment.as_ref() {
                Some(payment) => payment
                    .payees()
                    .into_iter()
                    .filter(|peer_id| peer_id != &network.peer_id())
                    .collect(),
                None => match network.get_replicate_candidates(data_addr.clone()).await {
                    Ok(peers) => peers,
                    Err(err) => {
                        error!("Replicating fresh record {pretty_key:?} get_replicate_candidates errored: {err:?}");
                        return;
                    }
                },
            };

            let our_peer_id = network.peer_id();
            let our_address = NetworkAddress::from_peer(our_peer_id);
            let keys = vec![(data_addr, data_type, validation_type.clone(), payment)];

            for peer_id in replicate_candidates {
                debug!("Replicating fresh record {pretty_key:?} to {peer_id:?}");
                let request = Request::Cmd(Cmd::FreshReplicate {
                    holder: our_address.clone(),
                    keys: keys.clone(),
                });

                network.send_req_ignore_reply(request, peer_id);
            }
            debug!(
                "Completed replicate fresh record {pretty_key:?} on store, in {:?}",
                start.elapsed()
            );
        });
    }

    // To fetch a received fresh record replication
    pub(crate) fn fresh_replicate_to_fetch(
        &self,
        holder: NetworkAddress,
        keys: Vec<(
            NetworkAddress,
            DataTypes,
            ValidationType,
            Option<ProofOfPayment>,
        )>,
    ) {
        let node = self.clone();
        let _handle = spawn(async move {
            let mut new_keys = vec![];
            for (addr, data_type, val_type, payment) in keys {
                if let Some(payment) = payment {
                    // Payment must be valid
                    match node
                        .payment_for_us_exists_and_is_still_valid(&addr, data_type, payment)
                        .await
                    {
                        Ok(_) => {}
                        Err(err) => {
                            info!("ProofOfPayment of {addr:?} is invalid with error {err:?}");
                            continue;
                        }
                    }
                } else {
                    // Must have existing copy
                    match node
                        .validate_key_and_existence(&addr, &addr.to_record_key())
                        .await
                    {
                        Ok(true) => {}
                        Ok(false) => {
                            info!(
                                "Received a fresh update against a non-existing record of {addr:?}"
                            );
                            continue;
                        }
                        Err(err) => {
                            info!("Failed to verify the local existence of {addr:?} with error {err:?}");
                            continue;
                        }
                    }
                }
                new_keys.push((addr, val_type));
            }

            if !new_keys.is_empty() {
                // Adding to the replication_fetcher for the rate_limit purpose,
                // instead of fetching directly. To reduce potential choking risk.
                info!(
                    "Adding {} fresh records from {holder:?} to the replication_fetcher",
                    new_keys.len()
                );
                node.network()
                    .add_fresh_records_to_the_replication_fetcher(holder, new_keys);
            }
        });
    }
}

// Copyright 2024 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

#[macro_use]
extern crate tracing;

pub use evmlib::common::Address as RewardsAddress;
pub use evmlib::common::QuotePayment;
pub use evmlib::common::{QuoteHash, TxHash};
pub use evmlib::utils;
pub use evmlib::wallet::Error as EvmWalletError;
pub use evmlib::wallet::Wallet as EvmWallet;
pub use evmlib::Network as EvmNetwork;

mod amount;
mod data_payments;
mod error;

/// EVM network configuration
pub mod evm;

pub use data_payments::{PaymentQuote, ProofOfPayment, QuotingMetrics};

/// Types used in the public API
pub use amount::{Amount, AttoTokens};
pub use error::{EvmError, Result};

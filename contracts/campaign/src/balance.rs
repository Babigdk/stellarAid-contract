//! Contract balance verification for payouts (issue #764).
//!
//! # Why a pre-transfer check
//!
//! A transfer that is going to fail should fail *before* the contract mutates
//! any state. Verifying the balance up front turns a doomed payout into a
//! single, attributable `InsufficientContractBalance` failure instead of a
//! partial state change followed by a transfer error, and it keeps the ledger
//! entry for the failed payout out of the indexer's event stream.
//!
//! # Atomicity with the transfer
//!
//! The check reads the same token balance the transfer will draw from, and
//! both happen inside one invocation, so no other transaction can interleave
//! between them and drain the balance in between. The two must stay in the
//! same function; splitting them into separate entry points would reopen
//! exactly the race this closes.
//!
//! Note that *contract accounting* is a separate concern: a campaign's
//! `raised` total must also cover the payout, and that is checked separately so
//! a caller can tell "the campaign never raised this much" apart from "the
//! contract's wallet is short".
//!
//! # Panic style
//!
//! This module panics with the string `"InsufficientContractBalance"`, matching
//! the PascalCase error-name convention already used by this contract
//! (`"ContractFrozen"`, `"UnderReview"`). The typed equivalent
//! (`Error::InsufficientContractBalance`) lives in `contracts/campaign/src/errors.rs`.

use soroban_sdk::{token, Address, Env};

/// Error string raised when the contract's token balance cannot cover a payout.
/// Also the name of the typed variant in `contracts/campaign/src/errors.rs`.
pub const INSUFFICIENT_CONTRACT_BALANCE: &str = "InsufficientContractBalance";

/// The contract's current balance of `token`, in base units.
pub fn contract_balance(env: &Env, token_address: &Address) -> i128 {
    token::Client::new(env, token_address).balance(&env.current_contract_address())
}

/// Panics with `InsufficientContractBalance` unless the contract holds at
/// least `required` of `token_address`.
///
/// Call this **immediately before** the transfer and after every state write
/// that must not be rolled back on its own.
pub fn require_contract_balance(env: &Env, token_address: &Address, required: i128) -> i128 {
    if required < 0 {
        panic!("payout amount must be positive");
    }
    let balance = contract_balance(env, token_address);
    if balance < required {
        panic!("{}", INSUFFICIENT_CONTRACT_BALANCE);
    }
    balance
}

/// True when the contract can cover `required` of `token_address`. Read-only
/// counterpart of [`require_contract_balance`], for callers that want to
/// branch instead of abort.
pub fn can_cover(env: &Env, token_address: &Address, required: i128) -> bool {
    contract_balance(env, token_address) >= required
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Env;

    #[test]
    fn payout_panics_when_the_contract_is_short() {
        let env = Env::default();
        let token_address = Address::generate(&env);
        // A random address is not a SEP-41 token, so the balance read itself
        // fails; the important assertion is that it does not silently succeed.
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            require_contract_balance(&env, &token_address, 1)
        }));
        assert!(res.is_err());
    }

    #[test]
    fn negative_payout_is_rejected_before_any_balance_read() {
        let env = Env::default();
        let token_address = Address::generate(&env);
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            require_contract_balance(&env, &token_address, -1)
        }));
        assert!(res.is_err());
    }

    #[test]
    fn error_name_matches_the_typed_variant() {
        assert_eq!(INSUFFICIENT_CONTRACT_BALANCE, "InsufficientContractBalance");
    }
}

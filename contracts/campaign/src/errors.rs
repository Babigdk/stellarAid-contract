//! Comprehensive error taxonomy for the campaign contract (issue #759).
//!
//! # Code banding
//!
//! `contracts/shared/src/errors.rs` reserves `300–399` for the campaign
//! contract. This enum occupies that band starting at `301`, so a caller that
//! switches on the numeric code can tell a campaign error apart from an escrow
//! (`100–199`), commission (`200–299`) or donation (`400–499`) error without
//! knowing which contract produced it.
//!
//! # Stability
//!
//! Per `docs/UPGRADE_AND_ROLLBACK.md`, error code values are permanent. New
//! conditions get the next free code in the band; existing codes are never
//! renumbered or reused.
//!
//! # Shape
//!
//! Every variant carries a one-line description, surfaced three ways:
//!
//! * `Error::description()` — `&'static str`, for documentation and tooling.
//! * `Display` — human-readable text used in logs.
//! * `get_suggestion()` — a `Symbol` shortcode (≤9 chars, the `symbol_short!`
//!   limit) that a client can display without parsing English.

use soroban_sdk::{contracterror, panic_with_error, symbol_short, Env, Symbol};

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// The contract has already been initialized and cannot be re-initialized.
    AlreadyInitialized = 301,
    /// A state-changing entry point was called before `initialize`.
    NotInitialized = 302,
    /// The caller did not authenticate as the address authorized for the action.
    Unauthorized = 303,
    /// The campaign is closed (completed or cancelled) and accepts no further writes.
    CampaignEnded = 304,
    /// The campaign is not in a state that permits this operation.
    CampaignNotActive = 305,
    /// The donation asset is not on the campaign's accepted-asset list.
    AssetNotAccepted = 306,
    /// The donation is below the campaign's configured minimum.
    DonationTooSmall = 307,
    /// The contract's recorded balance cannot cover the requested payout.
    InsufficientContractBalance = 308,
    /// The campaign goal is zero or negative.
    InvalidGoalAmount = 309,
    /// The campaign end time is in the past or beyond the allowed horizon.
    InvalidEndTime = 310,
    /// An asset code or asset descriptor is malformed or over-length.
    InvalidAssets = 311,
    /// The campaign still holds raised funds, so it cannot be cancelled.
    CannotCancelWithFunds = 312,
    /// The refund window for this campaign has closed.
    RefundWindowClosed = 313,
    /// A dispute was raised too recently; the cooldown has not elapsed.
    DisputeCooldownActive = 314,
    /// A dispute is already open for this campaign.
    DisputeAlreadyOpen = 315,
    /// No dispute is open for this campaign.
    NoActiveDispute = 316,
    /// A withdrawal was requested but its notice window has not elapsed.
    WithdrawalWindowNotElapsed = 317,
    /// No pending withdrawal exists for this campaign.
    NoPendingWithdrawal = 318,
    /// An arithmetic operation would overflow or underflow.
    Overflow = 319,
    /// A re-entrant call was rejected by the reentrancy guard.
    Reentrant = 320,
    /// The contract is frozen in an emergency; all state changes are blocked.
    ContractFrozen = 321,
}

impl Error {
    /// One-line description of the condition, for documentation and tooling.
    pub const fn description(&self) -> &'static str {
        match self {
            Self::AlreadyInitialized => "contract has already been initialized",
            Self::NotInitialized => "contract has not been initialized",
            Self::Unauthorized => "caller is not authorized to perform this action",
            Self::CampaignEnded => "campaign is closed and accepts no further writes",
            Self::CampaignNotActive => "campaign is not in a state that permits this action",
            Self::AssetNotAccepted => "donation asset is not accepted by this campaign",
            Self::DonationTooSmall => "donation amount is below the minimum accepted",
            Self::InsufficientContractBalance => "contract balance cannot cover the requested payout",
            Self::InvalidGoalAmount => "campaign goal must be greater than zero",
            Self::InvalidEndTime => "campaign end time is invalid or outside the allowed horizon",
            Self::InvalidAssets => "asset code or asset descriptor is malformed",
            Self::CannotCancelWithFunds => "campaign still holds raised funds and cannot be cancelled",
            Self::RefundWindowClosed => "the refund window for this campaign has closed",
            Self::DisputeCooldownActive => "dispute cooldown has not elapsed since the last dispute",
            Self::DisputeAlreadyOpen => "a dispute is already open for this campaign",
            Self::NoActiveDispute => "no dispute is open for this campaign",
            Self::WithdrawalWindowNotElapsed => "withdrawal notice window has not elapsed",
            Self::NoPendingWithdrawal => "no pending withdrawal exists for this campaign",
            Self::Overflow => "arithmetic operation would overflow or underflow",
            Self::Reentrant => "re-entrant call rejected",
            Self::ContractFrozen => "contract is frozen; all state-changing operations are blocked",
        }
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.write_str(self.description())
    }
}

/// Short, client-displayable code for an error. Capped at 9 characters because
/// `symbol_short!` literals may not exceed that.
pub fn get_suggestion(error: Error) -> Symbol {
    match error {
        Error::AlreadyInitialized => symbol_short!("ALREADY"),
        Error::NotInitialized => symbol_short!("NO_INIT"),
        Error::Unauthorized => symbol_short!("AUTH"),
        Error::CampaignEnded => symbol_short!("ENDED"),
        Error::CampaignNotActive => symbol_short!("BAD_STS"),
        Error::AssetNotAccepted => symbol_short!("BAD_AST"),
        Error::DonationTooSmall => symbol_short!("TOO_SML"),
        Error::InsufficientContractBalance => symbol_short!("NO_FUND"),
        Error::InvalidGoalAmount => symbol_short!("BAD_GOAL"),
        Error::InvalidEndTime => symbol_short!("BAD_TIME"),
        Error::InvalidAssets => symbol_short!("BAD_AST"),
        Error::CannotCancelWithFunds => symbol_short!("HOLD_FND"),
        Error::RefundWindowClosed => symbol_short!("RFND_SHUT"),
        Error::DisputeCooldownActive => symbol_short!("DS_COOLD"),
        Error::DisputeAlreadyOpen => symbol_short!("DUP_DS"),
        Error::NoActiveDispute => symbol_short!("NO_DS"),
        Error::WithdrawalWindowNotElapsed => symbol_short!("EARLY"),
        Error::NoPendingWithdrawal => symbol_short!("NO_PEND"),
        Error::Overflow => symbol_short!("OVERFL"),
        Error::Reentrant => symbol_short!("REENTRY"),
        Error::ContractFrozen => symbol_short!("FROZEN"),
    }
}

/// Panics with `error`, aborting the whole invocation.
///
/// Wraps `soroban_sdk::panic_with_error!` so call sites read as ordinary
/// statements and every failure in this crate is guaranteed to be a typed
/// contract error rather than a free-form string.
#[inline]
pub fn raise(env: &Env, error: Error) -> ! {
    panic_with_error!(env, error)
}

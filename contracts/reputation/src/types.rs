use soroban_sdk::{contracttype, Address, Bytes, String};

/// The current state of a review in the moderation system.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewStatus {
    Active = 0,
    UnderReview = 1,
    Removed = 2,
    Cleared = 3,
}

/// Reason category supplied when reporting a review.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReportReason {
    Spam = 0,
    Abuse = 1,
    Misleading = 2,
    Other = 3,
}

/// Current state of an appeal filed against a moderation decision.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppealStatus {
    Pending = 0,
    Upheld = 1,
    Denied = 2,
    Escalated = 3,
}

/// A review submitted by a client about an artist.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewRecord {
    pub review_id: Bytes,
    pub artist: Address,
    pub reviewer: Address,
    pub rating_x10: u32,
    pub comment: String,
    pub status: ReviewStatus,
    pub created_ledger: u32,
}

/// A moderation report filed against a review.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReportRecord {
    pub review_id: Bytes,
    pub reporter: Address,
    pub reason: ReportReason,
    pub details: String,
    pub created_ledger: u32,
}

/// An appeal record filed by the artist or reviewer.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppealRecord {
    pub review_id: Bytes,
    pub appellant: Address,
    pub reason: String,
    pub status: AppealStatus,
    pub created_ledger: u32,
}

/// A moderation action taken by the admin on a review.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModerationRecord {
    pub review_id: Bytes,
    pub admin: Address,
    pub new_status: ReviewStatus,
    pub notes: String,
    pub ledger: u32,
}

/// Storage data keys.
#[contracttype]
pub enum DataKey {
    Admin,
    Review(Bytes),
    Report(Bytes, u32),
    ReportCount(Bytes),
    Appeal(Bytes),
    ModerationEntry(Bytes, u32),
    ModerationCount(Bytes),
    QueueEntry(u32),
    QueueSize,
}


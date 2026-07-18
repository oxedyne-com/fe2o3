use crate::{
    base::index::{
        BotPoolInd,
        WorkerInd,
        ZoneInd,
    },
    bots::worker::bot::WorkerType,
};
use oxedyne_fe2o3_core::{
    prelude::*,
    rand::RanDef,
};
use oxedyne_fe2o3_jdat::{
    id::IdDat,
    kind::Kind,
    usr::UsrKindId,
};

use std::{
    fmt::{
        self,
        Debug,
        Display,
    },
    mem,
};

/// Underlying integer type for a bot identifier.
pub type BidTyp = u64;
/// Length in bytes of a bot identifier.
pub const BID_LEN: usize = mem::size_of::<BidTyp>();
/// A random bot identifier, unique per bot instance.
pub type Bid = IdDat<{ BID_LEN }, BidTyp>;

/// Underlying integer type used to identify a database request.
pub type TicketTyp = u64;
// A random ticket that tags a database request so that its responses can be
// correlated back to the originating call.
new_type!(Ticket, TicketTyp, Clone, Copy, Default, Eq, Ord, PartialEq, PartialOrd);

impl fmt::Display for Ticket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:x?}", self.0)
    }
}
    
impl fmt::Debug for Ticket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Ticket({:x?})", self.0)
    }
}

impl Ticket {
    /// Creates a new random ticket.
    pub fn new() -> Self {
        Self(TicketTyp::randef())
    }
    /// Creates the zero ticket, used where no correlation is required.
    pub fn zero() -> Self {
        Self(0)
    }
}

/// Identifies a bot within the database by its role and location.
///
/// Every worker bot carries its random [`Bid`] together with the zone and
/// bot-pool index it occupies; the singleton bots (master, supervisor, config)
/// carry only a [`Bid`].
#[derive(Clone, Eq, Hash, PartialEq)]
pub enum OzoneBotId {
    /// A cache bot, in a given zone and pool position.
    CacheBot(Bid, ZoneInd, BotPoolInd),
    /// The configuration bot.
    ConfigBot(Bid),
    /// A file bot, in a given zone and pool position.
    FileBot(Bid, ZoneInd, BotPoolInd),
    /// An initialisation and garbage-collection bot, in a given zone and pool position.
    InitGarbageBot(Bid, ZoneInd, BotPoolInd),
    /// The master bot held by the database owner.
    Master(Bid),
    /// A reader bot, in a given zone and pool position.
    ReaderBot(Bid, ZoneInd, BotPoolInd),
    /// A server bot, in a given pool position (server bots are not zoned).
    ServerBot(Bid, BotPoolInd),
    /// The supervisor bot.
    Supervisor(Bid),
    /// A writer bot, in a given zone and pool position.
    WriterBot(Bid, ZoneInd, BotPoolInd),
    /// A zone bot, responsible for a given zone.
    ZoneBot(Bid, ZoneInd),
}

impl Debug for OzoneBotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CacheBot(bid, zind, bpind)        => write!(f, "CacheBot:{}:{}:{}", zind, bpind, bid),
            Self::ConfigBot(bid)                    => write!(f, "ConfigBot:{}", bid),
            Self::FileBot(bid, zind, bpind)         => write!(f, "FileBot:{}:{}:{}", zind, bpind, bid),
            Self::InitGarbageBot(bid, zind, bpind)  => write!(f, "InitGarbageBot:{}:{}:{}", zind, bpind, bid),
            Self::Master(bid)                       => write!(f, "Master:{}", bid),
            Self::ReaderBot(bid, zind, bpind)       => write!(f, "ReaderBot:{}:{}:{}", zind, bpind, bid),
            Self::ServerBot(bid, bpind)             => write!(f, "ServerBot:{}:{}", bpind, bid),
            Self::Supervisor(bid)                   => write!(f, "Supervisor:{}", bid),
            Self::WriterBot(bid, zind, bpind)       => write!(f, "WriterBot:{}:{}:{}", zind, bpind, bid),
            Self::ZoneBot(bid, zind)                => write!(f, "ZoneBot:{}:{}", zind, bid),
        }
    }
}

impl Display for OzoneBotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl OzoneBotId {
    /// Returns the bot identifier common to every variant.
    pub fn bid(&self) -> Bid {
        match self {
            Self::CacheBot(bid, ..)         => *bid,
            Self::ConfigBot(bid)            => *bid,
            Self::FileBot(bid, ..)          => *bid,
            Self::InitGarbageBot(bid, ..)   => *bid,
            Self::Master(bid)               => *bid,
            Self::ReaderBot(bid, ..)        => *bid,
            Self::ServerBot(bid, ..)        => *bid,
            Self::Supervisor(bid)           => *bid,
            Self::WriterBot(bid, ..)        => *bid,
            Self::ZoneBot(bid, ..)          => *bid,
        }
    }
    /// Constructs a fresh worker bot identifier of the given type at the given
    /// worker index, generating a new random [`Bid`].
    pub fn new_worker(wtyp: &WorkerType, wind: &WorkerInd) -> Self {
        match wtyp {
            WorkerType::Cache       => Self::CacheBot(Bid::randef(), *wind.zind(), *wind.bpind()),
            WorkerType::File        => Self::FileBot(Bid::randef(), *wind.zind(), *wind.bpind()),
            WorkerType::InitGarbage => Self::InitGarbageBot(Bid::randef(), *wind.zind(), *wind.bpind()),
            WorkerType::Reader      => Self::ReaderBot(Bid::randef(), *wind.zind(), *wind.bpind()),
            WorkerType::Writer      => Self::WriterBot(Bid::randef(), *wind.zind(), *wind.bpind()),
        }
    }
}

/// Returns the custom `Dat` user-kind identifier tagging a stored user record.
pub fn usr_kind_id_user() -> UsrKindId {
    UsrKindId::new(
        64_000,
        Some("USER"),
        Some(Kind::U128),
    )
}

/// Returns the custom `Dat` user-kind identifier used as a deletion tombstone.
pub fn usr_kind_id_deleted() -> UsrKindId {
    UsrKindId::new(
        64_100,
        Some("DELETED"),
        Some(Kind::Empty),
    )
}

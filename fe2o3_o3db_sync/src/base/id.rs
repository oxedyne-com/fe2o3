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

pub type BidTyp = u64;
pub const BID_LEN: usize = mem::size_of::<BidTyp>();
pub type Bid = IdDat<{ BID_LEN }, BidTyp>;

/// Used to identify a database request.
pub type TicketTyp = u64;
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
    pub fn new() -> Self {
        Self(TicketTyp::randef())
    }
    pub fn zero() -> Self {
        Self(0)
    }
}

#[derive(Clone, Eq, Hash, PartialEq)]
pub enum OzoneBotId {
    CacheBot(Bid, ZoneInd, BotPoolInd),
    ConfigBot(Bid),
    FileBot(Bid, ZoneInd, BotPoolInd),
    InitGarbageBot(Bid, ZoneInd, BotPoolInd),
    Master(Bid),
    ReaderBot(Bid, ZoneInd, BotPoolInd),
    ServerBot(Bid, BotPoolInd),
    Supervisor(Bid),
    WriterBot(Bid, ZoneInd, BotPoolInd),
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

pub fn usr_kind_id_user() -> UsrKindId {
    UsrKindId::new(
        64_000,
        Some("USER"),
        Some(Kind::U128),
    )
}

pub fn usr_kind_id_deleted() -> UsrKindId {
    UsrKindId::new(
        64_100,
        Some("DELETED"),
        Some(Kind::Empty),
    )
}

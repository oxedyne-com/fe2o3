use crate::srv::{
    cfg::ServerConfig,
    constant,
    guard::{
        addr::{
            AddressGuard,
            AddressLog,
        },
        data::{
            AddressData,
            UserData,
        },
        user::{
            UserGuard,
            UserLog,
        },
    },
    msg::{
        external::{
            MsgAssembler,
            MsgAssemblyParams,
            MsgBuilder,
            MsgState,
        },
        internal::{
            ServerMsg,
        },
    },
    packet::PacketValidator,
    pow::{
        DifficultyParams,
        PowPristine,
    },
    schemes::{
        WireSchemes,
        WireSchemesInput,
    },
    server::{
        RxEnv,
        ServerBot,
    },
};

use oxedize_fe2o3_bot::{
    bot::Bot,
};
use oxedize_fe2o3_core::{
    prelude::*,
    alt::Alt,
    channels::{
        simplex,
        Simplex,
    },
    thread::{
        Sentinel,
        thread_channel,
    },
};
use oxedize_fe2o3_crypto::{
    sign::SignatureScheme,
};
use oxedize_fe2o3_data::ring::RingTimer;
use oxedize_fe2o3_iop_crypto::{
    enc::Encrypter,
    sign::{
        Signer,
        SignerDefAlt,
    },
};
use oxedize_fe2o3_jdat::{
    cfg::Config,
    chunk::ChunkConfig,
    file::JdatMapFile,
    id::{
        IdDat,
        NumIdDat,
    },
};
use oxedize_fe2o3_hash::{
    hash::{
        HasherDefAlt,
        HashScheme,
    },
    map::ShardMap,
    pow::{
        PowCreateParams,
        PowVars,
        ProofOfWork,
    },
};
use oxedize_fe2o3_iop_hash::{
    api::{
        Hasher,
        HashForm,
    },
    csum::Checksummer,
};
use oxedize_fe2o3_syntax::core::SyntaxRef;

use std::{
    collections::BTreeMap,
    marker::PhantomData,
    net::{
        IpAddr,
        SocketAddr,
        UdpSocket,
    },
    path::Path,
    sync::{
        Arc,
        Mutex,
        RwLock,
    },
    time::{
        Duration,
        Instant,
        SystemTime,
        UNIX_EPOCH,
    },
};

use local_ip_address::local_ip;



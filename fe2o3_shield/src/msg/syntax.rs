//!
//!```ignore
//!
//!   Handshake messages - all single-packet
//!   --------------------------------------
//!   HReq1 = first handshake request
//!   HReq2 = second handshake request
//!   HReq3 = third and final handshake request                                                                      
//!   HResp1 = first handshake response                                                                              
//!   HResp2 = second handshake response                                                                             
//!   HResp3 = third and final handshake response                                                                    
//!                                                                                                                  
//!                           PEER X                                         PEER Y                                  
//!                          ========                                       ========                                 
//!                             |  Sign request with:                           |      zy is the global,             
//!                             |   sqx = signature private key                 |      non-specific required         
//!                             |  In packet or meta:                           |      number of zero bits           
//!                             |   ax = src_addr                               |      and is generally            
//!                             |   ux = uid                                    |      relatively large.           
//!                             |  Assumed or known:                            |                                  
//!                             |   cy = code expected by y                     |                                          
//!                             |   zy = zbits expected by y                    |                                  
//!                             |   spy = signature public key for y            |                                  
//!                             |                                               |        potentially drop          
//!    "Can I start an          +>>>>>>>>>>>>>>>>>>>>> HReq1 >>>>>>>>>>>>>>>>>>>|-----#  based on                  
//!    encrypted session        |                                               |        (ax, ux, cy, zy, spy)     
//!    with you?"               |  I'm sending you:                             |                                  
//!                             |  Plain in message:                            |      If the packet signature                                  
//!                             |   zxy = zbits expected from y                 |      verifies, we know the        
//!                             |   spy = last sign pub key I have for you      |      request is authentic.        
//!    zxy ~ zy                 |  In validation artefact:                      |      However, we may not
//!    (i.e. relatively large)  |   spx = my current signature pk               |      recognise the public key
//!                             |                                               |      spx. In this case, send X
//!                             |                                               |      the spx we have, and ask
//!                             |                                               |      them to sign the next
//!                             |                                               |      request with it.
//!                             |                Sign request with:             |      
//!                             |                 sqy = signature private key*  |      * Either current or sqy  
//!                             |                In packet or meta:             |      corresponding with spy   
//!                             |                 ay = src_addr                 |      sent by X.               
//!                             |                 uy = uid                      |                      
//!                             |                Using:                         |      ** Only send if spy sent                          
//!                             |                 cxy = code expected by y      |      by X is old.              
//!                             |                 zxy = zbits expected by y     |       
//!                             |                Optional request:              |                               
//!                             |                 sign HReq2 using old key      |       
//!                             |                                               |       
//!     drop due to      #------+<<<<<<<<<<<<<<<<<<<< HResp1 <<<<<<<<<<<<<<<<<<<+      "Ok, here is my
//!     (ay, uy, cy, zy)?       |                                               |      authentic response."
//!                             |                I'm sending you:               |      
//!                             |                Plain in message:              |      If spy sent by X is a
//!                             |                 cyx = code expected from x    |      valid old signature pk
//!                             |                 zyx = zbits expected from x   |      of mine, use it to sign,
//!                             |                 spx = your old sign pk (opt)**|      but also send current
//!                             |                                               |      spy.
//!    If the signature         |                                               |
//!    verifies we know the     |                                               |
//!    response is authentic.   |                                               |
//!                             |  Sign request with:                           |
//!                             |   sqx = old/curr sign private                 |
//!                             |  Use:                                         |
//!                             |   cyx = code expected by y                    |
//!                             |   zyx = zbits expected by y                   |
//!                             |                                               |
//!    "Ok that looks           +>>>>>>>>>>>>>>>>>>>> HReq2 >>>>>>>>>>>>>>>>>>>>|-----> drop due to
//!    authentic, I've signed   |                                               |       (ax, ux, cx, zx)?
//!    this with my old         |  I'm sending you, if necessary:               |
//!    signature if you didn't  |   spx = sign pub key used                     |
//!    recognise my current     |                                               |
//!    signature."              |                                               |
//!                             |                                               |
//!                             |                                               |
//!                             |                Sign request with:             |
//!                             |                 sqy = curr sign priv key      |     The request is authentic,
//!                             |                In packet or meta:             |     generate a KEM key pair
//!                             |                 ay = src_addr                 |     and send a random secret
//!                             |                 uy = uid                      |     session key.
//!                             |                Using:                         |
//!                             |                 cxy = code expected by y      |     No change to pow reqs, yet.
//!                             |                 zxy = zbits expected by y     |
//!                             |                Optional request:              |
//!                             |                                               |
//!     drop due to      #------+<<<<<<<<<<<<<<<<<<<< HResp2 <<<<<<<<<<<<<<<<<<<+     "Great, I've used the KEM 
//!     (ay, uy, cy, zy)?       |                                               |     for this protocol version
//!                             |                I'm sending you:               |     to send you a secret
//!                             |                 cyx = code expected from x    |     session encryption key."
//!                             |                 zyx = zbits expected from x   |
//!                             |                 ek = session enc key (enc)    |     The session id is always
//!                             |                 sid = session id, encrypted   |     encrypted.
//!                             |                                               |
//!    "I've encrypted a hash   +>>>>>>>>>>>>>>>>>>>> HReq3 >>>>>>>>>>>>>>>>>>>>|-----> drop due to
//!    of the session id."      |                                               |       (ax, ux, cx, zx)?
//!                             |   I'm sending you:                            |
//!                             |     H(sid) = session id, encrypted            |
//!                             |                                               |
//!                             |                                               |
//!     drop due to      #------+<<<<<<<<<<<<<<<<<<<< HResp3 <<<<<<<<<<<<<<<<<<<+     "Looks good, confirmed
//!     (ay, uy, cy, zy)?       |                                               |     from my end, you can begin
//!                             |                                               |     sending session messages."
//!                             |                                               |
//!     zxy relaxed as trust    |                                               |     zyx relaxed as trust
//!     increases during        |                                               |     increases during session.
//!     session.                |                                               |
//!                             |                                               |
//!                             |                                               |
//!                             +>>>>>>>>>>>>>>> Protocol message >>>>>>>>>>>>>>|-----> drop due to
//!                             |                                               |       (ax, ux, cx, zx)?
//!                             |   I'm sending you:                            |
//!                             |    cxy = updated code expected from y         |
//!                             |    zxy = updated zbits expected from y        |
//!                             |                                               |
//!                             |                                               |
//!                             |                                               |
//!                             |                                               |
//!     drop due to      #------+<<<<<<<<<<<<<<< Protocol message <<<<<<<<<<<<<<+
//!     (ay, uy, cy, zy)?       |                                               |
//!                             |                I'm sending you:               |
//!                             |                 cyx = updated code expected   |
//!                             |                       from x                  |
//!                             |                 zyx = updated zbits expected  |
//!                             |                       from x                  |
//!                             |                                               |
//!                             |                                               |
//!                             |                                               |
//!     
//!```
use crate::{
    cfg::ShieldConfig,
    constant,
    msg::external::{
        HandshakeType,
        IdentifiedMessage,
        Message,
        MsgBuilder,
        MsgType,
    },
    guard::data::AddressData,
};

use oxedize_fe2o3_core::{
    prelude::*,
    byte::{
        Encoding,
        IntoBytes,
    },
    mem::Extract,
};
use oxedize_fe2o3_iop_crypto::sign::Signer;
use oxedize_fe2o3_jdat::{
    prelude::*,
    try_extract_dat_as,
    id::{
        IdDat,
        NumIdDat,
    },
};
use oxedize_fe2o3_hash::{
    pow::{
        Pristine,
        ZeroBits,
    },
};
use oxedize_fe2o3_iop_hash::api::Hasher;
use oxedize_fe2o3_syntax::{
    msg::{
        Msg as SyntaxMsg,
        MsgCmd as SyntaxMsgCmd,
    },
    arg::{
        Arg,
        ArgConfig,
    },
    cmd::{
        Cmd,
        CmdConfig,
    },
    core::{
        Syntax,
        SyntaxRef,
        SyntaxConfig,
    },
};
use oxedize_fe2o3_text::string::Stringer;

use std::{
    net::{
        SocketAddr,
        UdpSocket,
    },
};

pub fn empty() -> Syntax {
    Syntax::from(SyntaxConfig {
        name:   fmt!("Shield Protocol"),
        ver:    constant::VERSION.clone(),
        about:  Some(fmt!("Signature and Hash In Every Little Datagram (SHIELD)")),
        ..Default::default()
    })
}

pub fn build() -> Outcome<Syntax>
{
    let mut p = empty();

    // Reusable components ====================================================
    //
    //let arg_uid = Arg::from(ArgConfig {
    //    name:   fmt!("IdDat"),
    //    hyph1:  fmt!("u"),
    //    hyph2:  Some(fmt!("uid")),
    //    evals:  vec![UID::KIND],
    //    help:   Some(fmt!("User identifier (unsigned int)")),
    //    ..Default::default()
    //});
    let arg_sid = Arg::from(ArgConfig {
        name:   fmt!("IdDat"),
        hyph1:  fmt!("s"),
        hyph2:  Some(fmt!("sid")),
        vals:   vec![(constant::SESSION_ID_KIND, fmt!("Id (unsigned int)"))],
        help:   Some(fmt!("Session identifier")),
        ..Default::default()
    });
    //let arg_pow_code = Arg::from(ArgConfig {
    //    name:   fmt!("PowCode"),
    //    hyph1:  fmt!("pc"),
    //    hyph2:  Some(fmt!("pow-code")),
    //    evals:  vec![Kind::BC64],
    //    help:   Some(fmt!("Use this proof of work code for packets")),
    //    ..Default::default()
    //});
    let arg_pow_zbits = Arg::from(ArgConfig {
        name:   fmt!("PowZeroBits"),
        hyph1:  fmt!("zb"),
        hyph2:  Some(fmt!("zero-bits")),
        vals:   vec![(Kind::U16, fmt!("Number of zero bits (u16)"))],
        help:   Some(fmt!("Number of zero bits to use for proof of work")),
        ..Default::default()
    });
    //let arg_my_pack_sign_pk = Arg::from(ArgConfig {
    //    name:   fmt!("MyPacketPublicSigningKey"),
    //    hyph1:  fmt!("mpsp"),
    //    hyph2:  Some(fmt!("my-pack-sign-pk")),
    //    evals:  vec![Kind::BC64],
    //    help:   Some(fmt!("My packet public signing key")),
    //    ..Default::default()
    //});
    let arg_your_pack_sign_pk = Arg::from(ArgConfig {
        name:   fmt!("YourPacketPublicSigningKey"),
        hyph1:  fmt!("yppsk"),
        hyph2:  Some(fmt!("your-pack-sign-pk")),
        vals:   vec![(Kind::BC64, fmt!("Public key"))],
        help:   Some(fmt!("Your packet public signing key")),
        ..Default::default()
    });
    //let arg_my_msg_sign_pk = Arg::from(ArgConfig {
    //    name:   fmt!("MyMessagePublicSigningKey"),
    //    hyph1:  fmt!("mmsp"),
    //    hyph2:  Some(fmt!("my-msg-sign-pk")),
    //    evals:  vec![Kind::BC64],
    //    help:   Some(fmt!("My message public signing key")),
    //    ..Default::default()
    //});
    //let arg_your_msg_sign_pk = Arg::from(ArgConfig {
    //    name:   fmt!("YourMessagePublicSigningKey"),
    //    hyph1:  fmt!("ymsp"),
    //    hyph2:  Some(fmt!("your-msg-sign-pk")),
    //    evals:  vec![Kind::BC64],
    //    help:   Some(fmt!("Your message public signing key")),
    //    ..Default::default()
    //});
    //let arg_sign = Arg::from(ArgConfig {
    //    name:   fmt!("Signature"),
    //    hyph1:  fmt!("sig"),
    //    hyph2:  Some(fmt!("signature")),
    //    evals:  vec![Kind::BC64],
    //    help:   Some(fmt!("Signature applied to message contents")),
    //    ..Default::default()
    //});

    // Message arguments ======================================================
    //
    //p = res!(p.add_arg(arg_uid.clone().required(true)));
    //p = res!(p.add_arg(arg_pow_zbits.clone().required(true)));
    //p = res!(p.add_arg(arg_pow_code.clone().required(true)));
    p = res!(p.add_arg(arg_sid.clone().required(false)));
    p = res!(p.add_arg(arg_pow_zbits.clone().required(true)));

    // HReq1 ==================================================================
    //
    let mut c = Cmd::from(CmdConfig {
        name:   fmt!("hreq1"),
        help:   Some(fmt!("Initial handshake request")),
        ..Default::default()
    });
    c = res!(c.add_arg(arg_your_pack_sign_pk.clone())); // My version of your signature public key.
    p = res!(p.add_cmd(c));

    // HResp1 =================================================================
    //
    let mut c = Cmd::from(CmdConfig {
        name:   fmt!("hresp1"),
        help:   Some(fmt!("Initial handshake response")),
        ..Default::default()
    });
    //let arg_send_sign_pk = Arg::from(ArgConfig {
    //    name:   fmt!("SendPublicSigningKey"),
    //    hyph1:  fmt!("sspk"),
    //    hyph2:  Some(fmt!("send-sign-pk")),
    //    help:   Some(fmt!("Send signing public key")),
    //    ..Default::default()
    //});
    c = res!(c.add_arg(arg_pow_zbits.clone().required(true)));
    c = res!(c.add_arg(arg_your_pack_sign_pk)); // My version of your signature public key.
    p = res!(p.add_cmd(c));

    // HReq2 ==================================================================
    //
    let c = Cmd::from(CmdConfig {
        name:   fmt!("hreq2"),
        help:   Some(fmt!("Second handshake request")),
        ..Default::default()
    });
    //c = res!(c.add_arg(arg_sign_pk.clone().required(false)));
    //c = res!(c.add_arg(arg_sign.clone().required(true)));
    p = res!(p.add_cmd(c));

    // HResp2 =================================================================
    //
    let mut c = Cmd::from(CmdConfig {
        name:   fmt!("hresp2"),
        help:   Some(fmt!("Second handshake response")),
        ..Default::default()
    });
    let arg_skey_enc = Arg::from(ArgConfig {
        name:   fmt!("EncSymKey"),
        hyph1:  fmt!("sk"),
        hyph2:  Some(fmt!("sym-key")),
        vals:   vec![(Kind::BC64, fmt!("Private key"))],
        help:   Some(fmt!("Encrypted symmetric encryption key for session")),
        ..Default::default()
    });
    c = res!(c.add_arg(arg_skey_enc.required(true)));
    p = res!(p.add_cmd(c));

    Ok(p)

}

/// The MsgFmt captures the syntax protocol against which incoming and outgoing messages are
/// validated, and the encoding for any outgoing messages.
#[derive(Clone, Debug, Default)]
pub struct MsgFmt {
    pub syntax:     SyntaxRef,
    pub encoding:   Encoding,
}

/// Capture the required (when receiving) and expected (when sending) Proof of Work parameters.
#[derive(Clone, Debug, Default)]
pub struct MsgPow {
    pub zbits:  ZeroBits,
    //pub code:   Option<[u8; constant::POW_CODE_LEN]>,  
}

impl MsgPow {

    pub fn from_msg(msg: &mut SyntaxMsg) -> Outcome<Self> {
        let zbits = match msg.get_arg_vals_mut("-zb") {
            Some(v) => try_extract_dat_as!(v[0].extract(), ZeroBits, U8, U16, U32),
            None => return Err(err!(
                "No proof of work zero bits specified in message arguments (-zb).";
                Input, Missing)),
        };
        //let code = match msg.get_arg_vals_mut("-pc") {
        //    Some(v) => {
        //        let v = try_extract_dat!(v[0].extract(), BC64);
        //        Some(res!(<[u8; constant::POW_CODE_LEN]>::try_from(&v[..]),
        //            Decode, Bytes))
        //    },
        //    None => None,
        //};
        Ok(Self {
            zbits,
            //code,
        })
    }
}

/// Capture the user id and possibly the session id.
#[derive(Clone, Debug, Default)]
pub struct MsgIds<
    const SIDL: usize,
    const UIDL: usize,
    SID:    NumIdDat<SIDL>,
    UID:    NumIdDat<UIDL>,
> {
    pub sid_opt:    Option<IdDat<SIDL, SID>>,
    pub uid:        IdDat<UIDL, UID>,
}

impl<
    const SIDL: usize,
    const UIDL: usize,
    SID:    NumIdDat<SIDL>,
    UID:    NumIdDat<UIDL>,
>
    MsgIds<SIDL, UIDL, SID, UID>
{
    pub fn from_msg(uid: IdDat<UIDL, UID>, msg: &mut SyntaxMsg) -> Outcome<Self> {
        //let uid = match msg.get_arg_vals_mut("-u") {
        //    Some(v) => try_extract_dat_as!(v[0].extract(), IdDat, U128),
        //    None => return Err(err!(
        //        "No user id value in message arguments (-u).",
        //    ), Input, Missing)),
        //};
        let sid_opt = match msg.get_arg_vals_mut("-s") {
            Some(v) => Some(res!(IdDat::<SIDL, SID>::from_dat(v[0].extract()))),
            None => None, // not required
        };
        Ok(Self {
            uid,
            sid_opt,
        })
    }
}

/// Rather than a generic and possibly more complex callback mechanism, the processing of server
/// command is customised so as to access only parameters needed from the server loop scope.
/// Incoming server commands are encoded in a `oxedize_fe2o3_syntax::msg::MsgCmd` using the `Syntax`
/// defined in `oxedize_fe2o3_shield::syntax`.  Each must be associated with a `struct` below that
/// is accessed in `oxedize_fe2o3_o3db::bots::bot_server`.  This must capture some basic information (i.e.
/// `MsgFmt` and `MsgIds`) as well as the command-specific data.  The associated `struct` must have
/// its own custom method for processing the incoming command (e.g.
/// `oxedize_fe2o3_o3db::comm::wire::HReq1::process`), and should implement `ServerCommand` in order to
/// access supporting methods.  There are plenty of examples to copy and modify.
pub trait ServerCommand<
    const SIDL: usize,
    const UIDL: usize,
    SID: NumIdDat<SIDL>,
    UID: NumIdDat<UIDL>,
>:
    Default + IdentifiedMessage + IntoBytes
{
    fn fmt(&self)       -> &MsgFmt;
    fn pow(&self)       -> &MsgPow;
    fn mid(&self)       -> &MsgIds<SIDL, UIDL, SID, UID>;
    fn syntax(&self)    -> &SyntaxRef           { &self.fmt().syntax }
    fn encoding(&self)  -> &Encoding            { &self.fmt().encoding }
    fn uid(&self)       -> IdDat<UIDL, UID>    { self.mid().uid.clone() }
    fn sid_opt(&self)   -> Option<IdDat<SIDL, SID>> {
        self.mid().sid_opt.as_ref().clone().copied()
    }
    fn pow_zbits(&self) -> ZeroBits { self.pow().zbits }
    fn pad_last(&self)  -> bool     { true }
    //fn pow_code(&self)  -> Option<[u8; constant::POW_CODE_LEN]> { self.pow().code }
    fn inc_sigpk(&self) -> bool; // Include signature public key in outgoing validator?
    fn deconstruct(&mut self, _mcmd: &mut SyntaxMsgCmd) -> Outcome<()> { Ok(()) }
    fn construct(self)  -> Outcome<SyntaxMsg>;

    fn build<
        const MIDL: usize,
        MID: NumIdDat<MIDL>,
        // Proof of work validator.
        H: Hasher + Send + 'static, // Proof of work hasher.
        //const N: usize, // Pristine + Nonce size.
        const P0: usize, // Length of pristine prefix bytes (i.e. not included in artefact).
        const P1: usize, // Length of pristine bytes (i.e. included in artefact).
        PRIS: Pristine<P0, P1>, // Pristine supplied to hasher.
        // Digital signature validation.
        S: Signer,
    >(
        self,
        builder: &MsgBuilder<H, P0, P1, PRIS, S>,
    )
        -> Outcome<Vec<Vec<u8>>>
    {
        // Copy some self parameters before consumption by into_bytes
        let msg_name = self.name();
        let msg_typ = self.typ();
        //let uid = res!(self.uid().to_bytes(Vec::new()));
        let inc_sigpk = self.inc_sigpk();
        let pad_last = self.pad_last();

        let uid = self.uid().clone();

        let msg_byts = res!(self.into_bytes(Vec::new()));
        //let tstamp = res!(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)).as_secs();

        let (packets, warning) = res!(Message::create::<
            MIDL,
            UIDL,
            MID,
            UID,
            H,
            {constant::POW_INPUT_LEN},
            P0,
            P1,
            PRIS,
            S,
        >(
            msg_name,
            msg_byts,
            // Metadata
            msg_typ,
            &constant::VERSION,
            uid,
            //tstamp,
            &ShieldConfig::chunker(builder.chunk_cfg.clone().set_pad_last(pad_last)),
            &builder.validator,
            &builder.powparams,
            inc_sigpk,
        ));
        if let Some(warning) = warning {
            warn!("{}", warning);
        }
        Ok(packets)
    }

    //fn send<
    //    // Proof of work validator.
    //    H: Hasher + Send + 'static, // Proof of work hasher.
    //    //const N: usize, // Pristine + Nonce size.
    //    const P0: usize, // Length of pristine prefix bytes (i.e. not included in artefact).
    //    const P1: usize, // Length of pristine bytes (i.e. included in artefact).
    //    PRIS: Pristine<P0, P1>, // Pristine supplied to hasher.
    //    // Digital signature validation.
    //    S: Signer,
    //>(
    //    self,
    //    builder: &MsgBuilder<H, P0, P1, PRIS, S>,
    //)
    //    -> Outcome<()>
    //{
    //    // Copy some self parameters before consumption by into_bytes
    //    let msg_name = self.name();
    //    let msg_typ = self.typ();
    //    //let uid = res!(self.uid().to_bytes(Vec::new()));
    //    let inc_sigpk = self.inc_sigpk();
    //    let pad_last = self.pad_last();

    //    let uid = self.uid().clone();

    //    let msg_byts = res!(self.into_bytes(Vec::new()));
    //    //let tstamp = res!(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)).as_secs();

    //    let (packets, warning) = res!(Message::create::<
    //        //{constant::USER_ID_BYTE_LEN},
    //        H,
    //        {constant::POW_INPUT_LEN},
    //        P0,
    //        P1,
    //        PRIS,
    //        S,
    //    >(
    //        msg_name,
    //        msg_byts,
    //        // Metadata
    //        msg_typ,
    //        &constant::VERSION,
    //        uid,
    //        //tstamp,
    //        &Config::chunker(builder.chunk_cfg.clone().set_pad_last(pad_last)),
    //        &builder.validator,
    //        &builder.powparams,
    //        inc_sigpk,
    //    ));
    //    if let Some(warning) = warning {
    //        warn!("{}", warning);
    //    }
    //    res!(Message::send(
    //        &builder.src_sock,
    //        &builder.trg_addr,
    //        packets,
    //    ));
    //    Ok(())
    //}

    fn send_udp(
        src_sock:   &UdpSocket,
        trg_addr:   &SocketAddr,
        packets:    Vec<Vec<u8>>,
    )
        -> Outcome<()>
    {
        for packet in packets {
            res!(src_sock.send_to(&packet, &trg_addr));
        }
        Ok(())
    }
}

#[macro_export]
macro_rules! impl_into_bytes_for_server_msg {
    ($t:ty) => {
        impl IntoBytes for $t {
            fn into_bytes(self, buf: Vec<u8>) -> Outcome<Vec<u8>> {
                res!(self.construct()).into_bytes(buf)
            }
        }
    }
}

// HReq1 =======================================================================
/// Initiate an encrypted session.  X is the sender, Y the receiver.
///
/// X signs the request packet with their current signature, including the public key spx.  If
/// this fails to verify at Y, the packet will be dropped.  Peers do not keep a record of all old
/// peer keys, only the current version.  If Y has no existing record of spx, X is unknown to Y
/// and its policy regarding unknown users will determine whether the handshake continues.  If
/// the spx included by X does not match the version kept by Y, Y responds by sending the old spx
/// and asking for an authentic signature in HReq2.
///
/// Y is free to set its difficulty and code requirements for incoming proofs of work.  For
/// example, required difficulty could increase quickly and significantly  when the incoming
/// request rate becomes exceptional, suggesting a possible DOS attack.  Y can also distribute a
/// new code via a side channel in such circumstances.  X is likewise free to choose its own
/// difficulty and code for the proof of work, but if Y considers these invalid, the request will
/// be silently dropped, and the address possibly blacklisted for an incorrect code.  If the code
/// is thought to be correct, X may continue to try, but in a rate limited way otherwise the
/// address will be blacklisted.  To minimise work and retries, peers determined to connect will
/// tend to choose a relatively high difficulty, disincentivising DOS attacks.
#[derive(Clone, Debug, Default)]
pub struct HReq1<
    const SIDL: usize,
    const UIDL: usize,
    SID:    NumIdDat<SIDL>,
    UID:    NumIdDat<UIDL>,
> {
    pub fmt: MsgFmt,
    pub pow: MsgPow,
    pub mid: MsgIds<SIDL, UIDL, SID, UID>,
    // Command-specific
    pub peer_sigpk: Option<Vec<u8>>, // Your version of my signature public key.
}
    
impl<
    const SIDL: usize,
    const UIDL: usize,
    SID:    NumIdDat<SIDL>,
    UID:    NumIdDat<UIDL>,
>
    IntoBytes for HReq1<SIDL, UIDL, SID, UID>
{
    fn into_bytes(self, buf: Vec<u8>) -> Outcome<Vec<u8>> {
        res!(self.construct()).into_bytes(buf)
    }
}

impl<
    const SIDL: usize,
    const UIDL: usize,
    SID:    NumIdDat<SIDL>,
    UID:    NumIdDat<UIDL>,
>
    IdentifiedMessage for HReq1<SIDL, UIDL, SID, UID>
{
    fn typ(&self) -> MsgType { HandshakeType::Req1 as MsgType }
    fn name(&self) -> &'static str { "hreq1" }
}

impl<
    const SIDL: usize,
    const UIDL: usize,
    SID:    NumIdDat<SIDL>,
    UID:    NumIdDat<UIDL>,
>
    ServerCommand<SIDL, UIDL, SID, UID> for HReq1<SIDL, UIDL, SID, UID>
{
    fn fmt(&self) -> &MsgFmt { &self.fmt }
    fn pow(&self) -> &MsgPow { &self.pow }
    fn mid(&self) -> &MsgIds<SIDL, UIDL, SID, UID> { &self.mid }
    fn inc_sigpk(&self) -> bool { true }
    fn pad_last(&self) -> bool { false }

    fn construct(self) -> Outcome<SyntaxMsg> {
        let mut msg = SyntaxMsg::new(self.syntax().clone()); // cloning ref
        msg.set_encoding(*self.encoding());
        if let Some(sid) = self.sid_opt() {
            msg = res!(msg.add_arg_val("-s", Some(res!(sid.to_dat()))));
        }
        msg = res!(msg.add_arg_val("-zb", Some(dat!(self.pow_zbits()))));
        let mut mcmd = res!(msg.new_cmd(self.name()));
        if let Some(sigpk) = &self.peer_sigpk {
            mcmd = res!(mcmd.add_arg_val("-yppsk", Some(dat!(sigpk.clone()))));
        }
        //mcmd = res!(mcmd.add_arg_val("-zb", Some(dat!(self.pow_zbits()))));
        msg = res!(msg.add_cmd(mcmd));
        for line in Stringer::new(fmt!("{:?}", msg)).to_lines("  ") {
            debug!("{}", line);
        }
        res!(msg.validate());
        Ok(msg)
    }
    
    fn deconstruct(
        &mut self,
        mcmd: &mut SyntaxMsgCmd,
    )
        -> Outcome<()>
    {
        self.peer_sigpk = match mcmd.get_arg_vals_mut("-yppsk") {
            Some(vals) => Some(try_extract_dat!(vals[0].extract(), BC64)),
            None => None,
        };
        //self.pow.zbits = match mcmd.get_arg_vals_mut("-zb") {
        //    Some(vals) => try_extract_dat_as!(vals[0].extract(), ZeroBits, U16),
        //    None => return Err(err!(errmsg!("HReq1: expected proof of work difficulty value."),
        //        Invalid, Input, Missing)),
        //};
        Ok(())
    }
}

impl<
    const SIDL: usize,
    const UIDL: usize,
    SID:    NumIdDat<SIDL>,
    UID:    NumIdDat<UIDL>,
>
    HReq1<SIDL, UIDL, SID, UID>
{
    pub fn send<
        const MIDL: usize,
        MID: NumIdDat<MIDL>,
        // Proof of work validator.
        H: Hasher + Send + 'static, // Proof of work hasher.
        //const N: usize, // Pristine + Nonce size.
        const P0: usize, // Length of pristine prefix bytes (i.e. not included in artefact).
        const P1: usize, // Length of pristine bytes (i.e. included in artefact).
        PRIS: Pristine<P0, P1>, // Pristine supplied to hasher.
        // Digital signature validation.
        S: Signer,
    >(
        syntax:     SyntaxRef,
        builder:    &MsgBuilder<H, P0, P1, PRIS, S>,
        _mid_opt:   Option<IdDat<MIDL, MID>>,
        sid_opt:    Option<IdDat<SIDL, SID>>,
        uid:        IdDat<UIDL, UID>,
    )
        -> Outcome<()>
    {
        let msg = Self { 
            fmt: MsgFmt {
                syntax,
                encoding: constant::DEFAULT_MSG_ENCODING, // TODO allow client to change
            },                                            
            mid: MsgIds {
                sid_opt,
                uid,
            },
            pow: MsgPow {
                zbits: builder.powparams.pvars.zbits,
            },
            ..Default::default()
        };
        let packets = res!(msg.build::<MIDL, MID, H, P0, P1, PRIS, S>(builder));
        <HReq1<SIDL, UIDL, SID, UID> as ServerCommand<SIDL, UIDL, SID, UID>>::send_udp(
            &builder.src_sock,
            &builder.trg_addr,
            packets,
        )
    }

    pub fn server_process(
        &mut self,
        mcmd:       &mut SyntaxMsgCmd,
        adata:      &mut AddressData, // For pow parameters.
        //mut udata:  &mut UserData<{ constant::POW_CODE_LEN }>, // For pow parameters.
        //ugrd:       &mut UserGuard<IdDat, UserData>, // For user signing pk.
        //// For sending HResp1.
        //src_addr:   &SocketAddr,
        //chunker:    Chunker,
        ////pack_size:  usize,
        //src_sock:   &UdpSocket,
        //trg_addr:   &SocketAddr,
    )
        -> Outcome<()>
    {
        res!(self.deconstruct(mcmd)); // We now have all command-specific data.
        adata.your_zbits = self.pow.zbits;
        debug!("Yay it worked!");
        //// Create a fresh pow code and assign to the source address.
        //let mut code = [0u8; constant::POW_CODE_LEN];
        //Rand::fill_u8(&mut code);
        //let pow_code = code.to_vec();
        //adata.apow_code = Some(pow_code.clone());
        //// Request transmission of signing key?
        //let req_send_key = match ugrd.get_user_log_mut(&self.uid()) {
        //    Some(ulog) => {
        //        if ulog.data.sigpk.is_none() {
        //            ulog.data.waiting_for_sigpk = true;
        //            true
        //        } else {
        //            ulog.data.waiting_for_sigpk = false;
        //            false
        //        }
        //    },
        //    None => true,
        //};
        //let response = HResp1 {
        //    fmt: self.fmt().clone(),
        //    pow: self.pow().clone(),
        //    mid: self.mid().clone(),
        //    req_send_key,
        //};
        //debug!("Sending hresp1: {}", res!(response.clone().construct()));
        //response.send(
        //    src_addr,
        //    chunker,
        //    &src_sock,
        //    &trg_addr,
        //)
        Ok(())
    }
}

// HResp1 ======================================================================
#[derive(Clone, Debug, Default)]
pub struct HResp1<
    const SIDL: usize,
    const UIDL: usize,
    SID:    NumIdDat<SIDL>,
    UID:    NumIdDat<UIDL>,
> {
    pub fmt:        MsgFmt,
    pub pow:        MsgPow,
    pub mid:        MsgIds<SIDL, UIDL, SID, UID>,
    // Command-specific
    pub send_key:   bool,
}
    
//impl_into_bytes_for_server_msg!(HResp1);
//
//impl IdentifiedMessage for HResp1 {
//    fn typ(&self) -> MsgType { HandshakeType::Response1 as MsgType }
//    fn name(&self) -> &'static str { "hresp1" }
//}
//
//impl ServerCommand for HResp1 {
//
//    fn fmt(&self) -> &MsgFmt { &self.fmt }
//    fn pow(&self) -> &MsgPow { &self.pow }
//    fn mid(&self) -> &MsgIds { &self.mid }
//
//    fn deconstruct(
//        &mut self,
//        mcmd: &mut SyntaxMsgCmd,
//    )
//        -> Outcome<()>
//    {
//        self.send_key = mcmd.has_arg("-sspk");
//        self.pow_code = match mcmd.get_arg_vals_mut("-pc") {
//            Some(vals) => try_extract_dat!(vals[0].extract(), BC64),
//            None => return Err(err!(errmsg!("No proof of work code found."),
//                Invalid, Input, Missing)),
//        };
//        self.pow_zbits = match mcmd.get_arg_vals_mut("-zb") {
//            Some(vals) => try_extract_dat_as!(vals[0].extract(), pow::ZeroBits, U16),
//            None => return Err(err!(errmsg!("No proof of work zero bit value found."),
//                Invalid, Input, Missing)),
//        };
//        Ok(())
//    }
//
//    fn construct(self) -> Outcome<SyntaxMsg> {
//        let mut msg = SyntaxMsg::new(self.syntax().clone()); // TODO do we have to clone here?
//        msg.set_encoding(*self.encoding());
//        msg = res!(msg.add_arg_val("-u", Some(dat!(self.uid()))));
//        let mut mcmd = res!(msg.new_cmd("hresp1"));
//        if self.send_key {
//            mcmd = res!(mcmd.add_arg("-sspk"));
//        }
//        mcmd = res!(mcmd.add_arg_val("-pc", Some(Daticle::BC64(self.pow_code))));
//        mcmd = res!(mcmd.add_arg_val("-zb", Some(dat!(self.pow_zbits))));
//        msg = res!(msg.add_cmd(mcmd));
//        res!(msg.validate());
//        Ok(msg)
//    }
//}
//
//impl HResp1 {
//
//    //pub fn client_process(
//    //    &mut self,
//    //    mcmd:     &mut SyntaxMsgCmd,
//    //    mut ugrd:   &mut UserGuard<IdDat, UserData>, // For user signing pk.
//    //    // For sending.
//    //    src_addr:   SocketAddr,
//    //    pack_size:  usize,
//    //    src_sock:   &UdpSocket,
//    //    trg_addr:   &SocketAddr,
//    //)
//    //    -> Outcome<()>
//    //{
//    //    res!(self.deconstruct(mcmd)); // We now have all command-specific data.
//
//    //    Ok(())
//    //}
//}
//
////// HReq2 =======================================================================
////#[derive(Debug, Default)]
////pub struct HReq2 {
////    pub fmt: MsgFmt,
////    pub mid: MsgIds,
////    // Command-specific
////    pub sigpk: Option<Vec<u8>>,
////}
////    
////impl_into_bytes_for_server_msg!(HReq2);
////
////impl IdentifiedMessage for HReq2 {
////    fn typ(&self) -> MsgType { HandshakeType::Request2 as MsgType }
////    fn name(&self) -> &'static str { "hreq2" }
////}
////
////impl Message for HReq2 {}
////
////impl ServerCommand for HReq2 {
////
////    fn fmt(&self) -> &MsgFmt { &self.fmt }
////    fn mid(&self) -> &MsgIds { &self.mid }
////
////    fn construct(self) -> Outcome<SyntaxMsg> {
////        let mut msg = SyntaxMsg::new(self.syntax().clone());
////        msg.set_encoding(*self.encoding());
////        let mut mcmd = res!(msg.new_cmd("hreq2"));
////        mcmd = res!(mcmd.add_arg_val("-pc", Daticle::BC64(self.pow_code)));
////        msg = res!(msg.add_cmd(mcmd));
////        res!(msg.validate());
////        Ok(msg)
////    }
////}
////
////impl HReq2 {
////
////    pub fn server_process(
////        &mut self,
////        mcmd:     &mut SyntaxMsgCmd,
////        mut adata:  &mut AddressData, // For pow parameters.
////        mut ugrd:   &mut UserGuard<IdDat, UserData>, // For user signing pk.
////        // For sending HResp1.
////        src_addr:   SocketAddr,
////        pack_size:  usize,
////        src_sock:   &UdpSocket,
////        trg_addr:   &SocketAddr,
////    )
////        -> Outcome<()>
////    {
////        res!(self.deconstruct(mcmd)); // We now have all command-specific data.
////        let waiting_for_sigpk = match ugrd.get_user_log_mut(&self.uid()) {
////            Some(ulog) => {
////                if ulog.data.sigpk.is_none() {
////                    ulog.data.waiting_for_sigpk = true;
////                    true
////                } else {
////                    ulog.data.waiting_for_sigpk = false;
////                    false
////                }
////            },
////            None => true,
////        };
////        let mut code = [0u8; PowPristine::CODE_LEN];
////        Rand::fill_u8(&mut code);
////        let pow_code = code.to_vec();
////        adata.apow_code = Some(pow_code.clone());
////        let send_key = match ugrd.get_user_log_mut(&self.uid()) {
////            Some(ulog) => ulog.data.sigpk.is_none(),
////            None => true,
////        };
////        let response = HResp1 {
////            fmt:        self.fmt().clone(),
////            mid:        self.mid().clone(),
////            send_key,
////            pow_code,
////            pow_zbits:  adata.apow_zbits,
////        };
////        response.send(
////            src_addr,
////            code,
////            adata.apow_zbits,
////            pack_size,
////            &src_sock,
////            &trg_addr,
////        )
////    }
////}

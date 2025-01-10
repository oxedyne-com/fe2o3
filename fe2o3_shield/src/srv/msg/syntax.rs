use crate::srv::{
    cfg::ServerConfig,
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

pub fn base_msg() -> Outcome<SyntaxRef> {

    let mut s = Syntax::from(SyntaxConfig {
        name:   fmt!("Shield Protocol"),
        ver:    constant::VERSION.clone(),
        about:  Some(fmt!("Signed Hash In Every Little Datagram (SHIELD)")),
        ..Default::default()
    });

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
    //s = res!(s.add_arg(arg_uid.clone().required(true)));
    //s = res!(s.add_arg(arg_pow_zbits.clone().required(true)));
    //s = res!(s.add_arg(arg_pow_code.clone().required(true)));
    s = res!(s.add_arg(arg_sid.clone().required(false)));
    s = res!(s.add_arg(arg_pow_zbits.clone().required(true)));

    // HReq1 ==================================================================
    //
    let mut c = Cmd::from(CmdConfig {
        name:   fmt!("hreq1"),
        help:   Some(fmt!("Initial handshake request")),
        ..Default::default()
    });
    c = res!(c.add_arg(arg_your_pack_sign_pk.clone())); // My version of your signature public key.
    s = res!(s.add_cmd(c));

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
    s = res!(s.add_cmd(c));

    // HReq2 ==================================================================
    //
    let c = Cmd::from(CmdConfig {
        name:   fmt!("hreq2"),
        help:   Some(fmt!("Second handshake request")),
        ..Default::default()
    });
    //c = res!(c.add_arg(arg_sign_pk.clone().required(false)));
    //c = res!(c.add_arg(arg_sign.clone().required(true)));
    s = res!(s.add_cmd(c));

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
    s = res!(s.add_cmd(c));

    Ok(SyntaxRef::new(s))
}
